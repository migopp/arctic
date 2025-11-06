use core::cmp;
use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::iter::Sort;
use crate::key::Read as _;
use crate::key::Write as _;
use crate::raw::edge;
use crate::raw::Edge;
use crate::Key;

pub(crate) enum RangeIter<'g, 'l, K: Key, C, S: Sort> {
    Root { key: K::Write, next: Option<u64> },
    Node(NodeIter<'g, 'l, K, C, S>),
}

impl<'g, 'l, K, C, S> RangeIter<'g, 'l, K, C, S>
where
    K: Key,
    S: Sort,
{
    pub(crate) unsafe fn new_unchecked(
        root: &'g Atomic128<Edge<C>>,
        prefix: K::Read<'l>,
        mut min: K::Read<'l>,
        mut max: K::Read<'l>,
    ) -> Self {
        validate!(min <= max);

        if matches!(S::compare(min, max), cmp::Ordering::Greater) {
            core::mem::swap(&mut min, &mut max);
        }

        let edge = root.load_packed(Ordering::Acquire);
        let bits = prefix.bits();
        let key = edge.meta().key();
        let mut writer = K::Write::from(prefix);

        writer.extend(bits, edge.meta().key());

        match edge.child() {
            None => Self::Root {
                key: writer,
                next: None,
            },
            Some(edge::Child::Value(value)) => {
                let reader = K::Read::from(&writer);
                if reader < K::reborrow(min) || reader > K::reborrow(max) {
                    return Self::Root {
                        key: writer,
                        next: None,
                    };
                }

                Self::Root {
                    key: writer,
                    next: Some(value),
                }
            }
            Some(edge::Child::Node(node)) => {
                let bits = bits + key.len().bits() as usize;
                let node = unsafe { node.into_ref_unchecked() };

                let reader = K::Read::from(&writer);
                validate!(matches!(
                    S::compare(reader, K::reborrow(min.slice(bits))),
                    cmp::Ordering::Equal | cmp::Ordering::Greater
                ));
                validate!(matches!(
                    S::compare(reader, K::reborrow(max.slice(bits))),
                    cmp::Ordering::Equal | cmp::Ordering::Less
                ));

                let first = (reader == K::reborrow(min.slice(bits))).then(|| min.get(bits));
                let last = (reader == K::reborrow(max.slice(bits))).then(|| max.get(bits));

                let mut stack = Vec::with_capacity(7);
                stack.push((bits, first, last, S::range(node, first, last)));

                Self::Node(NodeIter {
                    stack,
                    key: writer,
                    min,
                    max,
                    _sort: PhantomData,
                })
            }
        }
    }

    #[inline]
    pub(crate) fn for_each<F: FnMut(&K::Write, u64)>(self, mut apply: F) {
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
    pub(crate) fn lend(&mut self) -> Option<(&K::Write, u64)> {
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
    fn lend(&mut self) -> Option<(&K::Write, u64)> {
        self.walk::<true, _>(|_, _| ())
    }

    #[inline]
    fn for_each<F: FnMut(&K::Write, u64)>(&mut self, apply: F) {
        self.walk::<false, _>(apply);
    }

    #[inline]
    fn walk<const YIELD: bool, F: FnMut(&K::Write, u64)>(
        &mut self,
        mut apply: F,
    ) -> Option<(&K::Write, u64)> {
        'vertical: loop {
            let (bits, first, last, iter) = self.stack.last_mut()?;
            let bits = *bits;

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
                self.key.truncate(bits);
                self.key.push(bits, byte);

                unsafe {
                    // SAFETY: we just pushed `byte` onto `key`
                    self.key.extend_nonempty_unchecked(bits + 8, meta.key());
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
                            self.stack.push((
                                bits + 8 + meta.key().len().bits() as usize,
                                None,
                                None,
                                unsafe { S::range(node, None, None) },
                            ));
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
                                K::reborrow(self.min.slice(bits)),
                            ) {
                                cmp::Ordering::Less => continue 'horizontal,
                                cmp::Ordering::Equal => Some(self.min.get(bits)),
                                cmp::Ordering::Greater => None,
                            }
                        } else {
                            None
                        };

                        let last = if check_last {
                            match S::compare(
                                K::Read::from(&self.key),
                                K::reborrow(self.max.slice(bits)),
                            ) {
                                cmp::Ordering::Less => None,
                                cmp::Ordering::Equal => Some(self.max.get(bits)),
                                cmp::Ordering::Greater => {
                                    self.stack.clear();
                                    return None;
                                }
                            }
                        } else {
                            None
                        };

                        let node = unsafe { node.into_ref_unchecked() };
                        self.stack.push((
                            bits + 8 + meta.key().len().bits() as usize,
                            first,
                            last,
                            unsafe { S::range(node, first, last) },
                        ));
                        continue 'vertical;
                    }
                }
            }
        }
    }
}
