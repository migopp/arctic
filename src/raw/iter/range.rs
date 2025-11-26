use core::marker::PhantomData;
use core::ops::ControlFlow;
use core::sync::atomic::Ordering;

use ribbit::Atomic;

use crate::iter::Order;
use crate::raw;
use crate::raw::edge;
use crate::raw::iter::Lower as _;
use crate::raw::iter::Upper as _;
use crate::raw::key;
use crate::raw::node::Lower as _;
use crate::raw::node::Upper as _;
use crate::raw::Edge;

pub(crate) enum RangeIter<
    'g,
    R: key::Read,
    W: key::Write,
    M: ribbit::Pack<Packed: edge::Meta>,
    B: raw::iter::Range<R>,
    O,
> {
    Root { key: W, next: Option<u64> },
    Node(NodeIter<'g, R, W, M, B, O>),
}

impl<'g, R, W, M, B, O> Default for RangeIter<'g, R, W, M, B, O>
where
    R: key::Read<Edge = M>,
    W: key::Write<Edge = M>,
    W: From<R>,
    M: ribbit::Pack<Packed: edge::Meta>,
    B: raw::iter::Range<R>,
{
    fn default() -> Self {
        Self::Root {
            key: W::default(),
            next: None,
        }
    }
}

impl<'g, R, W, M, B, O> RangeIter<'g, R, W, M, B, O>
where
    R: key::Read<Edge = M>,
    W: key::Write<Edge = M>,
    W: From<R>,
    M: ribbit::Pack<Packed: edge::Meta>,
    B: raw::iter::Range<R>,
    O: Order,
{
    pub(crate) unsafe fn new_unchecked(root: &'g Atomic<Edge<M>>, prefix: R, range: B) -> Self {
        let edge = root.load_packed(Ordering::Acquire);

        let Some(child) = edge.child() else {
            return Self::default();
        };

        let key = edge.meta();
        let bits = prefix.bits();
        let mut writer = W::from(prefix);
        let bits = writer.write(W::len_from_bits(bits), key);

        let mut lower = range.lower();
        let mut upper = range.upper();

        let Some((lower_byte, upper_byte)) = lower.check(key).zip(upper.check(key)) else {
            return Self::default();
        };

        match child {
            edge::Child::Value(value) => Self::Root {
                key: writer,
                next: Some(value),
            },
            edge::Child::Node(node) => {
                let node = unsafe { node.into_ref_unchecked() };
                let mut stack = Vec::with_capacity(7);
                stack.push((bits, node.iter::<O, _, _>(lower_byte, upper_byte)));

                Self::Node(NodeIter {
                    lower,
                    upper,
                    stack,
                    key: writer,
                    _order: PhantomData,
                })
            }
        }
    }

    #[inline]
    pub(crate) fn for_each<F: FnMut(&W, u64) -> ControlFlow<()>>(self, mut apply: F) {
        match self {
            RangeIter::Root { key, mut next } => {
                crate::cold();
                if let Some(value) = next.take() {
                    let _ = apply(&key, value);
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

pub(crate) struct NodeIter<
    'g,
    R: key::Read,
    W: key::Write,
    M: ribbit::Pack<Packed: edge::Meta> + 'g,
    B: raw::iter::Range<R>,
    O,
> {
    lower: B::Lower,
    upper: B::Upper,
    key: W,
    stack: Vec<(
        W::Len,
        raw::node::NodeIter<
            'g,
            <B::Lower as raw::iter::Lower<R>>::Bound,
            <B::Upper as raw::iter::Upper<R>>::Bound,
            M,
        >,
    )>,
    _order: PhantomData<O>,
}

impl<'g, R, W, M, B, O> NodeIter<'g, R, W, M, B, O>
where
    R: key::Read<Edge = M>,
    W: key::Write<Edge = M>,
    M: ribbit::Pack<Packed: edge::Meta> + 'g,
    B: raw::iter::Range<R>,
    O: Order,
{
    #[inline]
    fn lend(&mut self) -> Option<(&W, u64)> {
        self.walk::<true, _>(|_, _| ControlFlow::Continue(()))
    }

    #[inline]
    fn for_each<F: FnMut(&W, u64) -> ControlFlow<()>>(&mut self, apply: F) {
        self.walk::<false, _>(apply);
    }

    #[inline]
    fn walk<const YIELD: bool, F: FnMut(&W, u64) -> ControlFlow<()>>(
        &mut self,
        mut apply: F,
    ) -> Option<(&W, u64)> {
        'vertical: loop {
            let (bits, iter) = self.stack.last_mut()?;
            let bits = *bits;

            'horizontal: loop {
                let Some((byte, edge)) = (if O::REVERSE {
                    iter.next_back()
                } else {
                    iter.next()
                }) else {
                    self.stack.pop();
                    continue 'vertical;
                };

                let edge = edge.load_packed(Ordering::Acquire);
                let Some(child) = edge.child() else {
                    continue 'horizontal;
                };

                let key = edge.meta();
                let bits = self.key.replace(bits, byte, key);

                let check_lower = iter.lower().check(byte);
                let check_upper = iter.upper().check(byte);

                if !check_lower && !check_upper {
                    match child {
                        edge::Child::Value(value) if YIELD => return Some((&self.key, value)),
                        edge::Child::Value(value) => match apply(&self.key, value) {
                            ControlFlow::Continue(()) => continue 'horizontal,
                            ControlFlow::Break(()) => {
                                self.stack.clear();
                                return None;
                            }
                        },
                        edge::Child::Node(node) => {
                            let node = unsafe { node.into_ref_unchecked() };
                            let lower = Default::default();
                            let upper = Default::default();
                            self.stack.push((bits, node.iter::<O, _, _>(lower, upper)));
                            continue 'vertical;
                        }
                    }
                }

                crate::cold();

                let lower = if check_lower {
                    match self.lower.check(key) {
                        Some(lower) => lower,
                        None if O::REVERSE => {
                            self.stack.clear();
                            return None;
                        }
                        None => continue 'horizontal,
                    }
                } else {
                    Default::default()
                };

                let upper = if check_upper {
                    match self.upper.check(key) {
                        Some(upper) => upper,
                        None if O::REVERSE => continue 'horizontal,
                        None => {
                            self.stack.clear();
                            return None;
                        }
                    }
                } else {
                    Default::default()
                };

                match child {
                    edge::Child::Value(value) if YIELD => {
                        return Some((&self.key, value));
                    }
                    edge::Child::Value(value) => match apply(&self.key, value) {
                        ControlFlow::Continue(()) => continue 'horizontal,
                        ControlFlow::Break(()) => {
                            self.stack.clear();
                            return None;
                        }
                    },
                    edge::Child::Node(node) => {
                        let node = unsafe { node.into_ref_unchecked() };
                        self.stack.push((bits, node.iter::<O, _, _>(lower, upper)));
                        continue 'vertical;
                    }
                }
            }
        }
    }
}
