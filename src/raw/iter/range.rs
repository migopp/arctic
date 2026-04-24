use core::cmp;
use core::marker::PhantomData;
use core::ops::ControlFlow;
use core::ops::RangeFrom;
use core::ops::RangeFull;
use core::ops::RangeInclusive;
use core::ops::RangeToInclusive;
use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use ribbit::Atomic;

use crate::Order;
use crate::raw;
use crate::raw::Edge;
use crate::raw::edge;
use crate::raw::edge::Key as _;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::node::Lower as _;
use crate::raw::node::Upper as _;

pub(crate) enum RangeIter<'g, K: key::Read, W: key::Write, R: Range<K>, O> {
    Root {
        writer: W,
        next: Option<(u64, NonNull<Atomic<Edge<K::Edge>>>)>,
    },
    Node(NodeIter<'g, K, W, R, O>),
}

impl<'g, K, W, R, O> Default for RangeIter<'g, K, W, R, O>
where
    K: key::Read,
    W: key::Write,
    R: Range<K>,
    O: Order,
{
    fn default() -> Self {
        Self::Root {
            writer: W::default(),
            next: None,
        }
    }
}

impl<'g, K, W, R, O> RangeIter<'g, K, W, R, O>
where
    K: key::Read,
    W: key::Write<Edge = K::Edge> + From<K>,
    R: Range<K>,
    O: Order,
{
    pub(crate) unsafe fn new_unchecked(
        root: NonNull<Atomic<Edge<K::Edge>>>,
        prefix: K,
        range: &R,
    ) -> Self {
        let edge = unsafe { root.as_ref() }.load_packed(Ordering::Acquire);

        let Some(child) = edge.child() else {
            return Self::default();
        };

        let key = edge.meta();
        let bits = prefix.bits();
        let mut writer = W::from(prefix);
        let len = writer.write(W::len_from_bits(bits), key);

        let mut lower = range.lower(bits);
        let mut upper = range.upper(bits);

        let Some((lower_byte, upper_byte)) = lower.check(key).zip(upper.check(key)) else {
            return Self::default();
        };

        match child {
            edge::Child::Value(value) => Self::Root {
                writer,
                next: Some((value, root)),
            },
            edge::Child::Node(node) => {
                let mut stack = Vec::with_capacity(7);
                stack.push((len, unsafe { node.entries(lower_byte, upper_byte) }));

                Self::Node(NodeIter {
                    lower,
                    upper,
                    writer,
                    stack,
                    _order: PhantomData,
                })
            }
        }
    }

    #[inline]
    pub(crate) fn for_each_internal<
        F: FnMut((&W, u64, NonNull<Atomic<Edge<K::Edge>>>)) -> ControlFlow<()>,
    >(
        self,
        mut apply: F,
    ) {
        match self {
            RangeIter::Root { writer, mut next } => {
                crate::cold();
                if let Some((value, edge)) = next.take() {
                    let _ = apply((&writer, value, edge));
                }
            }
            RangeIter::Node(mut iter) => iter.for_each_internal(apply),
        }
    }

    #[inline]
    pub(crate) fn lend(&mut self) -> Option<(&W, u64, NonNull<Atomic<Edge<K::Edge>>>)> {
        match self {
            RangeIter::Root { writer, next } => {
                crate::cold();
                let (value, edge) = next.take()?;
                Some((writer, value, edge))
            }
            RangeIter::Node(iter) => iter.lend(),
        }
    }
}

pub(crate) struct NodeIter<'g, K, W, R, O>
where
    K: key::Read,
    W: key::Write,
    R: Range<K>,
{
    lower: R::Lower,
    upper: R::Upper,
    writer: W,
    stack: Vec<(
        W::Len,
        raw::node::NodeIter<
            'g,
            <R::Lower as Lower<K::Edge>>::Bound,
            <R::Upper as Upper<K::Edge>>::Bound,
            K::Edge,
        >,
    )>,
    _order: PhantomData<O>,
}

impl<'g, K, W, R, O> NodeIter<'g, K, W, R, O>
where
    K: key::Read,
    R: Range<K>,
    W: key::Write<Edge = K::Edge>,
    O: Order,
{
    #[inline]
    fn lend(&mut self) -> Option<(&W, u64, NonNull<Atomic<Edge<K::Edge>>>)> {
        self.walk::<true, _>(|(_, _, _)| unreachable!())
    }

    #[inline]
    fn for_each_internal<F: FnMut((&W, u64, NonNull<Atomic<Edge<K::Edge>>>)) -> ControlFlow<()>>(
        &mut self,
        apply: F,
    ) {
        self.walk::<false, _>(apply);
    }

    #[inline]
    fn walk<
        const YIELD: bool,
        F: FnMut((&W, u64, NonNull<Atomic<Edge<K::Edge>>>)) -> ControlFlow<()>,
    >(
        &mut self,
        mut apply: F,
    ) -> Option<(&W, u64, NonNull<Atomic<Edge<K::Edge>>>)> {
        'vertical: loop {
            let (len, iter) = self.stack.last_mut()?;
            let len = *len;

            'horizontal: loop {
                let Some((mut byte, mut edge)) = (if O::ASCEND {
                    iter.next()
                } else {
                    iter.next_back()
                }) else {
                    self.stack.pop();
                    continue 'vertical;
                };

                let mut len = len;
                let mut check_lower = iter.lower().check(byte);
                let mut check_upper = iter.upper().check(byte);

                'compress: loop {
                    let (meta, child) = {
                        let edge = unsafe { edge.as_ref() }.load_packed(Ordering::Acquire);
                        let Some(child) = edge.child() else {
                            continue 'horizontal;
                        };
                        let meta = edge.meta();
                        (meta, child)
                    };

                    len = self.writer.replace(len, byte, meta);

                    let lower = if check_lower {
                        match self.lower.check(meta) {
                            Some(lower) => lower,
                            None if O::ASCEND => continue 'horizontal,
                            None => {
                                self.stack.clear();
                                return None;
                            }
                        }
                    } else {
                        Default::default()
                    };

                    let upper = if check_upper {
                        match self.upper.check(meta) {
                            Some(upper) => upper,
                            None if O::ASCEND => {
                                self.stack.clear();
                                return None;
                            }
                            None => continue 'horizontal,
                        }
                    } else {
                        Default::default()
                    };

                    match child {
                        edge::Child::Value(value) if YIELD => {
                            return Some((&self.writer, value, edge));
                        }
                        edge::Child::Value(value) => match apply((&self.writer, value, edge)) {
                            ControlFlow::Continue(()) => continue 'horizontal,
                            ControlFlow::Break(()) => {
                                self.stack.clear();
                                return None;
                            }
                        },
                        edge::Child::Node(node) => {
                            // Avoid pushing and popping iterators with only one child
                            match unsafe { node.entries(lower, upper) }.try_into_single() {
                                Ok((check_lower_, check_upper_, byte_, edge_)) => {
                                    check_lower = check_lower_;
                                    check_upper = check_upper_;
                                    byte = byte_;
                                    edge = edge_;
                                    continue 'compress;
                                }
                                Err(iter) => {
                                    self.stack.push((len, iter));
                                    continue 'vertical;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Copy, Clone)]
pub struct Include<T>(pub(crate) T);

pub struct Unbound<T = ()>(PhantomData<T>);

impl<T> Copy for Unbound<T> {}

impl<T> Clone for Unbound<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Default for Unbound<T> {
    #[inline]
    fn default() -> Self {
        Self(PhantomData)
    }
}

#[expect(private_bounds)]
pub trait Range<R>
where
    R: key::Read,
{
    #[expect(private_bounds)]
    type Lower: Lower<R::Edge>;

    #[expect(private_bounds)]
    type Upper: Upper<R::Edge>;

    fn lower(&self, bits: usize) -> Self::Lower;
    fn upper(&self, bits: usize) -> Self::Upper;

    #[inline]
    fn common_prefix(&self) -> R {
        R::default()
    }
}

impl<R: key::Read, T: Into<R> + Copy> Range<R> for RangeInclusive<T> {
    type Lower = Include<R>;
    type Upper = Include<R>;

    #[inline]
    fn lower(&self, bits: usize) -> Self::Lower {
        Include((*self.start()).into().suffix(bits))
    }

    #[inline]
    fn upper(&self, bits: usize) -> Self::Upper {
        Include((*self.end()).into().suffix(bits))
    }

    #[inline]
    fn common_prefix(&self) -> R {
        let lower = (*self.start()).into();
        let upper = (*self.end()).into();
        lower.common_prefix(upper)
    }
}

impl<R: key::Read, T: Into<R> + Copy> Range<R> for RangeFrom<T> {
    type Lower = Include<R>;
    type Upper = Unbound<R>;

    #[inline]
    fn lower(&self, bits: usize) -> Self::Lower {
        Include(self.start.into().suffix(bits))
    }

    #[inline]
    fn upper(&self, _bits: usize) -> Self::Upper {
        Unbound::default()
    }
}

impl<R: key::Read, T: Into<R> + Copy> Range<R> for RangeToInclusive<T> {
    type Lower = Unbound<R>;
    type Upper = Include<R>;

    #[inline]
    fn lower(&self, _bits: usize) -> Self::Lower {
        Unbound::default()
    }

    #[inline]
    fn upper(&self, bits: usize) -> Self::Upper {
        Include(self.end.into().suffix(bits))
    }
}

impl<K> Range<K> for RangeFull
where
    K: key::Read,
{
    type Lower = Unbound<K>;
    type Upper = Unbound<K>;

    #[inline]
    fn lower(&self, _: usize) -> Self::Lower {
        Unbound::default()
    }

    #[inline]
    fn upper(&self, _: usize) -> Self::Upper {
        Unbound::default()
    }
}

trait Lower<M>
where
    M: ribbit::Pack<Packed: edge::Meta>,
{
    type Bound: raw::node::Lower;

    fn check(&mut self, edge: ribbit::Packed<M>) -> Option<Self::Bound>;
}

trait Upper<M>
where
    M: ribbit::Pack<Packed: edge::Meta>,
{
    type Bound: raw::node::Upper;

    fn check(&mut self, edge: ribbit::Packed<M>) -> Option<Self::Bound>;
}

impl<R: key::Read> Lower<R::Edge> for Include<R> {
    type Bound = Option<u8>;

    fn check(&mut self, edge: ribbit::Packed<R::Edge>) -> Option<Self::Bound> {
        let key = edge.key();
        let len = key.len();

        // Skip check for fixed-length keys
        if R::BITS.is_none() && self.0.bits() < len.bits() {
            return None;
        }

        match self.0.read(len).cmp(&key) {
            cmp::Ordering::Less => Some(None),
            cmp::Ordering::Equal => Some(self.0.next()),
            cmp::Ordering::Greater => None,
        }
    }
}

impl<R: key::Read> Upper<R::Edge> for Include<R> {
    type Bound = Option<u8>;

    fn check(&mut self, edge: ribbit::Packed<R::Edge>) -> Option<Self::Bound> {
        let key = edge.key();
        let len = key.len();

        // Skip check for fixed-length keys
        if R::BITS.is_none() && self.0.bits() > len.bits() {
            return None;
        }

        match self.0.read(len).cmp(&key) {
            cmp::Ordering::Less => None,
            cmp::Ordering::Equal => Some(self.0.next()),
            cmp::Ordering::Greater => Some(None),
        }
    }
}

impl<R: key::Read> Lower<R::Edge> for Unbound<R> {
    type Bound = Unbound<R>;

    #[inline]
    fn check(&mut self, _: ribbit::Packed<R::Edge>) -> Option<Self::Bound> {
        Some(Unbound::default())
    }
}

impl<R: key::Read> Upper<R::Edge> for Unbound<R> {
    type Bound = Unbound<R>;

    #[inline]
    fn check(&mut self, _: ribbit::Packed<R::Edge>) -> Option<Self::Bound> {
        Some(Unbound::default())
    }
}
