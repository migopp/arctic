use core::cmp;
use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::iter::Sort;
use crate::key;
use crate::raw::edge;
use crate::raw::Edge;

pub(crate) enum RangeIter<'g, R, W, C, S: Sort> {
    Root { key: W, next: Option<u64> },
    Node(NodeIter<'g, R, W, C, S>),
}

impl<'g, R, W, C, S> RangeIter<'g, R, W, C, S>
where
    R: key::Read,
    W: key::Write,
    W: From<R>,
    S: Sort,
{
    pub(crate) unsafe fn new_unchecked(
        root: &'g Atomic128<Edge<C>>,
        prefix: R,
        mut min: R,
        mut max: R,
    ) -> Self {
        if S::REVERSE {
            core::mem::swap(&mut min, &mut max);
        }

        let edge = root.load_packed(Ordering::Acquire);
        let bits = prefix.bits();
        let key = edge.meta().key();
        let mut writer = W::from(prefix);
        writer.extend(bits, key);

        min.seek(prefix.bits());
        let min_len = key.len().min_bits(min.bits());
        let min_prefix = min.take(min_len);

        max.seek(prefix.bits());
        let max_len = key.len().min_bits(max.bits());
        let max_prefix = max.take(max_len);

        validate!(matches!(
            order::<S>(key.cmp(&min_prefix)),
            cmp::Ordering::Equal | cmp::Ordering::Greater
        ));

        validate!(matches!(
            order::<S>(key.cmp(&max_prefix)),
            cmp::Ordering::Equal | cmp::Ordering::Less
        ));

        match edge.child() {
            None => Self::Root {
                key: writer,
                next: None,
            },
            Some(edge::Child::Value(value)) => Self::Root {
                key: writer,
                next: Some(value),
            },
            Some(edge::Child::Node(node)) => {
                let node = unsafe { node.into_ref_unchecked() };

                let first = match key == min_prefix {
                    false => None,
                    true => min.next(),
                };

                let last = match key == max_prefix {
                    false => None,
                    true => max.next(),
                };

                let mut stack = Vec::with_capacity(7);
                stack.push((
                    bits + key.len().bits() as usize,
                    first,
                    last,
                    S::range(node, first, last),
                ));

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
    pub(crate) fn for_each<F: FnMut(&W, u64)>(self, mut apply: F) {
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
    pub(crate) fn lend(&mut self) -> Option<(&W, u64)> {
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

pub(crate) struct NodeIter<'g, R, W, C: 'g, S: Sort> {
    min: R,
    max: R,
    key: W,
    stack: Vec<(usize, Option<u8>, Option<u8>, S::RangeIter<'g, C>)>,
    _sort: PhantomData<S>,
}

impl<'g, R, W, C, S> NodeIter<'g, R, W, C, S>
where
    R: key::Read,
    W: key::Write,
    S: Sort,
{
    #[inline]
    fn lend(&mut self) -> Option<(&W, u64)> {
        self.walk::<true, _>(|_, _| ())
    }

    #[inline]
    fn for_each<F: FnMut(&W, u64)>(&mut self, apply: F) {
        self.walk::<false, _>(apply);
    }

    #[inline]
    fn walk<const YIELD: bool, F: FnMut(&W, u64)>(&mut self, mut apply: F) -> Option<(&W, u64)> {
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

                let key = edge.meta().key();
                self.key.truncate(bits);
                self.key.push(bits, byte);

                unsafe {
                    // SAFETY: we just pushed `byte` onto `key`
                    self.key.extend_nonempty_unchecked(bits + 8, key);
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
                                bits + 8 + key.len().bits() as usize,
                                None,
                                None,
                                unsafe { S::range(node, None, None) },
                            ));
                            continue 'vertical;
                        }
                    }
                }

                crate::cold();

                let first = if check_first {
                    let min_prefix = self.min.take(key.len().min_bits(self.min.bits()));
                    match order::<S>(key.cmp(&min_prefix)) {
                        cmp::Ordering::Less => continue 'horizontal,
                        cmp::Ordering::Equal => self.min.next(),
                        cmp::Ordering::Greater => None,
                    }
                } else {
                    None
                };

                let last = if check_last {
                    let max_prefix = self.max.take(key.len().min_bits(self.max.bits()));
                    match order::<S>(key.cmp(&max_prefix)) {
                        cmp::Ordering::Less => None,
                        cmp::Ordering::Equal => self.max.next(),
                        cmp::Ordering::Greater => {
                            self.stack.clear();
                            return None;
                        }
                    }
                } else {
                    None
                };

                match child {
                    edge::Child::Value(value) => {
                        if YIELD {
                            return Some((&self.key, value));
                        } else {
                            apply(&self.key, value);
                        }
                    }
                    edge::Child::Node(node) => {
                        let node = unsafe { node.into_ref_unchecked() };
                        self.stack.push((
                            bits + 8 + key.len().bits() as usize,
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

const fn order<S: Sort>(order: cmp::Ordering) -> cmp::Ordering {
    match S::REVERSE {
        false => order,
        true => order.reverse(),
    }
}
