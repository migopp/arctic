use core::cmp;
use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::cursor;
use crate::iter::Sort;
use crate::key::Read as _;
use crate::key::Write as _;
use crate::raw::edge;
use crate::raw::Edge;
use crate::Key;
use crate::Value;

pub(crate) enum RangeIter<'g, 'l, K: Key, V, S: Sort> {
    Root {
        key: K::Write,
        next: Option<ribbit::Packed<edge::Value<V>>>,
    },
    Node(NodeIter<'g, 'l, K, V, S>),
}

impl<'g, 'c, K, V, S> RangeIter<'g, 'c, K, V, S>
where
    K: Key,
    V: Value,
    S: Sort,
{
    pub(crate) fn new<'l>(
        cursor: &'c cursor::Prefix<
            'g,
            'l,
            K::Read<'l>,
            V,
            cursor::path::Hybrid<'g, K::Read<'l>, V>,
        >,
        min: K::Read<'l>,
        max: K::Read<'l>,
    ) -> Self {
        unsafe {
            Self::new_unchecked(
                cursor.edge(),
                <K::Write>::from(cursor.prefix()),
                K::reborrow(min),
                K::reborrow(max),
            )
        }
    }
}

impl<'g, 'l, K, V, S> RangeIter<'g, 'l, K, V, S>
where
    K: Key,
    S: Sort,
{
    pub(crate) unsafe fn new_unchecked(
        root: &'g Atomic128<Edge<V>>,
        mut key: K::Write,
        mut min: K::Read<'l>,
        mut max: K::Read<'l>,
    ) -> Self {
        validate!(min <= max);

        if matches!(S::compare(min, max), cmp::Ordering::Greater) {
            core::mem::swap(&mut min, &mut max);
        }

        let edge = root.load_packed(Ordering::Acquire);

        key.extend(edge.meta().key());

        match edge.child() {
            None => Self::Root { key, next: None },
            Some(edge::Child::Value(value)) => {
                let reader = K::Read::from(&key);
                if reader < K::reborrow(min) || reader > K::reborrow(max) {
                    return Self::Root { key, next: None };
                }

                Self::Root {
                    key,
                    next: Some(value),
                }
            }
            Some(edge::Child::Node(node)) => {
                let node = unsafe { node.into_ref_unchecked() };

                let reader = K::Read::from(&key);
                validate!(matches!(
                    S::compare(reader, K::reborrow(min.slice(key.bits()))),
                    cmp::Ordering::Equal | cmp::Ordering::Greater
                ));
                validate!(matches!(
                    S::compare(reader, K::reborrow(max.slice(key.bits()))),
                    cmp::Ordering::Equal | cmp::Ordering::Less
                ));

                let first =
                    (reader == K::reborrow(min.slice(key.bits()))).then(|| min.get(key.bits()));
                let last =
                    (reader == K::reborrow(max.slice(key.bits()))).then(|| max.get(key.bits()));

                let mut stack = Vec::with_capacity(7);
                stack.push((key.bits(), first, last, S::range(node, first, last)));

                Self::Node(NodeIter {
                    stack,
                    key,
                    min,
                    max,
                    _sort: PhantomData,
                })
            }
        }
    }

    #[inline]
    pub(crate) fn for_each<F: FnMut(&K::Write, ribbit::Packed<edge::Value<V>>)>(
        self,
        mut apply: F,
    ) {
        match self {
            RangeIter::Root { key, mut next } => {
                crate::cold();
                if let Some(value) = next.take() {
                    apply(&key, value);
                }
            }
            RangeIter::Node(mut iter) => iter.for_each(apply),
        }
    }

    #[inline]
    pub(crate) fn lend(&mut self) -> Option<(&K::Write, ribbit::Packed<edge::Value<V>>)> {
        match self {
            RangeIter::Root { key, next } => {
                crate::cold();
                let value = next.take()?;
                Some((key, value))
            }
            RangeIter::Node(iter) => iter.lend(),
        }
    }
}

pub(crate) struct NodeIter<'g, 'l, K: Key, V: 'g, S: Sort> {
    min: K::Read<'l>,
    max: K::Read<'l>,
    key: K::Write,
    stack: Vec<(usize, Option<u8>, Option<u8>, S::RangeIter<'g, V>)>,
    _sort: PhantomData<S>,
}

impl<'g, 'k, K, V, S> NodeIter<'g, 'k, K, V, S>
where
    K: Key,
    S: Sort,
{
    #[inline]
    fn lend(&mut self) -> Option<(&K::Write, ribbit::Packed<edge::Value<V>>)> {
        self.walk::<true, _>(|_, _| ())
    }

    #[inline]
    fn for_each<F: FnMut(&K::Write, ribbit::Packed<edge::Value<V>>)>(&mut self, apply: F) {
        self.walk::<false, _>(apply);
    }

    #[inline]
    fn walk<const YIELD: bool, F: FnMut(&K::Write, ribbit::Packed<edge::Value<V>>)>(
        &mut self,
        mut apply: F,
    ) -> Option<(&K::Write, ribbit::Packed<edge::Value<V>>)> {
        'vertical: loop {
            let (len, first, last, iter) = self.stack.last_mut()?;

            'horizontal: loop {
                let Some((byte, edge)) = iter.next() else {
                    self.stack.pop();
                    continue 'vertical;
                };

                let edge = edge.load_packed(Ordering::Acquire);
                let Some(child) = edge.child() else {
                    continue 'horizontal;
                };

                let meta = edge.meta();
                self.key.truncate(*len);
                self.key.push(byte);

                unsafe {
                    // SAFETY: we just pushed `byte` onto `key`
                    self.key.extend_nonempty_unchecked(meta.key());
                }

                let check_first = Some(byte) == *first;
                let check_last = Some(byte) == *last;

                if !check_first && !check_last {
                    match child {
                        edge::Child::Value(value) => {
                            if YIELD {
                                return Some((&self.key, value));
                            } else {
                                apply(&self.key, value);
                                continue 'horizontal;
                            }
                        }
                        edge::Child::Node(node) => {
                            let node = unsafe { node.into_ref_unchecked() };
                            self.stack.push((self.key.bits(), None, None, unsafe {
                                S::range(node, None, None)
                            }));
                            continue 'vertical;
                        }
                    }
                }

                crate::cold();

                match child {
                    edge::Child::Value(value) => {
                        if check_first && K::Read::from(&self.key) < K::reborrow(self.min) {
                            continue 'horizontal;
                        }

                        if check_last && K::Read::from(&self.key) > K::reborrow(self.max) {
                            self.stack.clear();
                            return None;
                        }

                        if YIELD {
                            return Some((&self.key, value));
                        } else {
                            apply(&self.key, value);
                        }
                    }
                    edge::Child::Node(node) => {
                        let first = if check_first {
                            match S::compare(
                                K::Read::from(&self.key),
                                K::reborrow(self.min.slice(self.key.bits())),
                            ) {
                                cmp::Ordering::Less => continue 'horizontal,
                                cmp::Ordering::Equal => Some(self.min.get(self.key.bits())),
                                cmp::Ordering::Greater => None,
                            }
                        } else {
                            None
                        };

                        let last = if check_last {
                            match S::compare(
                                K::Read::from(&self.key),
                                K::reborrow(self.max.slice(self.key.bits())),
                            ) {
                                cmp::Ordering::Less => None,
                                cmp::Ordering::Equal => Some(self.max.get(self.key.bits())),
                                cmp::Ordering::Greater => {
                                    self.stack.clear();
                                    return None;
                                }
                            }
                        } else {
                            None
                        };

                        let node = unsafe { node.into_ref_unchecked() };
                        self.stack.push((self.key.bits(), first, last, unsafe {
                            S::range(node, first, last)
                        }));
                        continue 'vertical;
                    }
                }
            }
        }
    }
}
