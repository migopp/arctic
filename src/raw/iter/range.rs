use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::iter::Order;
use crate::key;
use crate::raw;
use crate::raw::edge;
use crate::raw::iter::High as _;
use crate::raw::iter::Low as _;
use crate::raw::node::High as _;
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
        let edge = root.load_packed(Ordering::Acquire);

        let Some(child) = edge.child() else {
            return Self::default();
        };

        let bits = prefix.bits();
        let range = range.skip(bits);
        let mut lower = range.low();
        let mut upper = range.high();

        let key = edge.meta().key();
        let mut writer = W::from(prefix);
        let bits = writer.write(W::len_from_bits(bits), key);

        match child {
            edge::Child::Value(value) if lower.check_value(key) && upper.check_value(key) => {
                Self::Root {
                    key: writer,
                    next: Some(value),
                }
            }
            edge::Child::Value(_) => Self::default(),
            edge::Child::Node(node) => {
                let Some((lower_byte, upper_byte)) =
                    lower.check_node(key).zip(upper.check_node(key))
                else {
                    return Self::default();
                };

                let node = unsafe { node.into_ref_unchecked() };
                let mut stack = Vec::with_capacity(7);
                stack.push((bits, node.iter(lower_byte, upper_byte)));

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
    lower: B::Low,
    upper: B::High,
    key: W,
    stack: Vec<(
        W::Len,
        raw::node::SortedIter<
            'g,
            <B::Low as crate::raw::iter::Low<R>>::Bound,
            <B::High as crate::raw::iter::High<R>>::Bound,
            C,
        >,
    )>,
    _order: PhantomData<O>,
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

                let key = edge.meta().key();
                let bits = self.key.replace(bits, byte, key);

                let check_lower = iter.lower().is(byte);
                let check_upper = iter.upper().is(byte);

                if !check_lower && !check_upper {
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
                            self.stack.push((bits, node.iter(first, last)));
                            continue 'vertical;
                        }
                    }
                }

                crate::cold();

                match child {
                    edge::Child::Value(value) => {
                        if check_lower && !self.lower.check_value(key) {
                            if O::REVERSE {
                                self.stack.clear();
                                return None;
                            } else {
                                continue 'horizontal;
                            }
                        }

                        if check_upper && !self.upper.check_value(key) {
                            if O::REVERSE {
                                continue 'horizontal;
                            } else {
                                self.stack.clear();
                                return None;
                            }
                        }

                        if YIELD {
                            return Some((&self.key, value));
                        } else {
                            apply(&self.key, value);
                        }
                    }
                    edge::Child::Node(node) => {
                        let lower = if check_lower {
                            let Some(lower) = self.lower.check_node(key) else {
                                if O::REVERSE {
                                    self.stack.clear();
                                    return None;
                                } else {
                                    continue 'horizontal;
                                }
                            };
                            lower
                        } else {
                            Default::default()
                        };

                        let upper = if check_upper {
                            let Some(upper) = self.upper.check_node(key) else {
                                if O::REVERSE {
                                    continue 'horizontal;
                                } else {
                                    self.stack.clear();
                                    return None;
                                }
                            };
                            upper
                        } else {
                            Default::default()
                        };

                        let node = unsafe { node.into_ref_unchecked() };
                        self.stack.push((bits, node.iter(lower, upper)));
                        continue 'vertical;
                    }
                }
            }
        }
    }
}
