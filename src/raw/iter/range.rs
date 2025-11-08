use core::cmp;
use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::iter::Order;
use crate::key;
use crate::raw::edge;
use crate::raw::iter::Bound as _;
use crate::raw::Edge;

pub enum RangeIter<'g, R, W: key::Write, C, B: crate::raw::iter::Range_<R>, O: Order> {
    Root { key: W, next: Option<u64> },
    Node(NodeIter<'g, R, W, C, B, O>),
}

impl<'g, R, W, C, B, O> RangeIter<'g, R, W, C, B, O>
where
    R: key::Read,
    W: key::Write,
    W: From<R>,
    B: crate::raw::iter::Range_<R>,
    O: Order,
{
    pub(crate) unsafe fn new_unchecked(root: &'g Atomic128<Edge<C>>, prefix: R, range: B) -> Self {
        // if O::REVERSE {
        //     core::mem::swap(&mut min, &mut max);
        // }

        let edge = root.load_packed(Ordering::Acquire);
        let bits = prefix.bits();
        let key = edge.meta().key();

        let mut min = range.min();
        min.seek(bits);
        let Some(min_prefix) = min.take_min(key.len()) else {
            return Self::Root {
                key: W::default(),
                next: None,
            };
        };

        let mut max = range.max();
        max.seek(bits);
        let max_prefix = max.take_max(key.len());

        validate!(matches!(
            order::<O>(key.cmp(&min_prefix)),
            cmp::Ordering::Equal | cmp::Ordering::Greater
        ));

        validate!(matches!(
            order::<O>(key.cmp(&max_prefix)),
            cmp::Ordering::Equal | cmp::Ordering::Less
        ));

        let mut writer = W::from(prefix);
        let bits = writer.write(W::len_from_bits(bits), key);

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
                    true => min.next_min(),
                };

                let last = match key == max_prefix {
                    false => None,
                    true => max.next_max(),
                };

                let mut stack = Vec::with_capacity(7);
                stack.push((bits, first, last, O::range(node, first, last)));

                Self::Node(NodeIter {
                    min,
                    max,
                    stack,
                    key: writer,
                    _read: PhantomData,
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

pub(crate) struct NodeIter<'g, R, W: key::Write, C: 'g, B: crate::raw::iter::Range_<R>, O: Order> {
    min: B::Min,
    max: B::Max,
    key: W,
    stack: Vec<(W::Len, Option<u8>, Option<u8>, O::RangeIter<'g, C>)>,
    _read: PhantomData<R>,
    _sort: PhantomData<O>,
}

impl<'g, R, W, C, B, O> NodeIter<'g, R, W, C, B, O>
where
    R: key::Read,
    W: key::Write,
    B: crate::raw::iter::Range_<R>,
    O: Order,
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
                let bits = self.key.replace(bits, byte, key);

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
                            self.stack
                                .push((bits, None, None, unsafe { O::range(node, None, None) }));
                            continue 'vertical;
                        }
                    }
                }

                crate::cold();

                let first = if check_first {
                    let Some(min_prefix) = self.min.take_min(key.len()) else {
                        continue 'horizontal;
                    };

                    match order::<O>(key.cmp(&min_prefix)) {
                        cmp::Ordering::Less => continue 'horizontal,
                        cmp::Ordering::Equal => self.min.next_min(),
                        cmp::Ordering::Greater => None,
                    }
                } else {
                    None
                };

                let last = if check_last {
                    let max_prefix = self.max.take_max(key.len());

                    match order::<O>(key.cmp(&max_prefix)) {
                        cmp::Ordering::Less => None,
                        cmp::Ordering::Equal => self.max.next_max(),
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
                        self.stack
                            .push((bits, first, last, unsafe { O::range(node, first, last) }));
                        continue 'vertical;
                    }
                }
            }
        }
    }
}

const fn order<O: Order>(order: cmp::Ordering) -> cmp::Ordering {
    match O::REVERSE {
        false => order,
        true => order.reverse(),
    }
}
