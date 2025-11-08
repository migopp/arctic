use core::cmp;
use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::iter::Order;
use crate::key;
use crate::raw;
use crate::raw::edge;
use crate::raw::iter::High as _;
use crate::raw::iter::Low as _;
use crate::raw::node::High;
use crate::raw::node::Low as _;
use crate::raw::Edge;

pub enum RangeIter<'g, R, W: key::Write, C, B: crate::raw::iter::Range_<R>, O: Order> {
    Root { key: W, next: Option<u64> },
    Node(NodeIter<'g, R, W, C, B, O>),
}

impl<'g, R, W, C, B, O> Default for RangeIter<'g, R, W, C, B, O>
where
    R: key::Read,
    W: key::Write,
    W: From<R>,
    B: crate::raw::iter::Range_<R>,
    O: Order,
{
    fn default() -> Self {
        Self::Root {
            key: W::default(),
            next: None,
        }
    }
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

        let Some(child) = edge.child() else {
            return Self::default();
        };

        let bits = prefix.bits();
        let range = range.skip(bits);
        let mut min = range.low();
        let mut max = range.high();

        // validate!(matches!(
        //     order::<O>(key.cmp(&min_prefix)),
        //     cmp::Ordering::Equal | cmp::Ordering::Greater
        // ));
        //
        // validate!(matches!(
        //     order::<O>(key.cmp(&max_prefix)),
        //     cmp::Ordering::Equal | cmp::Ordering::Less
        // ));

        let key = edge.meta().key();
        let mut writer = W::from(prefix);
        let bits = writer.write(W::len_from_bits(bits), key);

        match child {
            edge::Child::Value(value) if min.check_value(key) && max.check_value(key) => {
                Self::Root {
                    key: writer,
                    next: Some(value),
                }
            }
            edge::Child::Value(_) => Self::default(),
            edge::Child::Node(node) => {
                let Some((first, last)) = min.check_node(key).zip(max.check_node(key)) else {
                    return Self::default();
                };

                let node = unsafe { node.into_ref_unchecked() };
                let mut stack = Vec::with_capacity(7);
                stack.push((bits, first, last, O::iter(node, first, last)));

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
    min: B::Low,
    max: B::High,
    key: W,
    stack: Vec<(
        W::Len,
        <B::Low as raw::iter::Low<R>>::Bound,
        <B::High as raw::iter::High<R>>::Bound,
        O::RangeIter<'g, C>,
    )>,
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

                let check_first = first.is(byte);
                let check_last = last.is(byte);

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
                            let first = Default::default();
                            let last = Default::default();
                            self.stack
                                .push((bits, first, last, unsafe { O::iter(node, first, last) }));
                            continue 'vertical;
                        }
                    }
                }

                crate::cold();

                match child {
                    edge::Child::Value(value) => {
                        if check_first && !self.min.check_value(key) {
                            continue 'horizontal;
                        }

                        if check_last && !self.max.check_value(key) {
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
                            let Some(first) = self.min.check_node(key) else {
                                continue 'horizontal;
                            };
                            first
                        } else {
                            Default::default()
                        };

                        let last = if check_last {
                            let Some(last) = self.max.check_node(key) else {
                                self.stack.clear();
                                return None;
                            };
                            last
                        } else {
                            Default::default()
                        };

                        let node = unsafe { node.into_ref_unchecked() };
                        self.stack
                            .push((bits, first, last, unsafe { O::iter(node, first, last) }));
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
