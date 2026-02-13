use core::cmp;
use core::ops::ControlFlow;
use core::ops::RangeFrom;
use core::ops::RangeFull;
use core::ops::RangeInclusive;
use core::ops::RangeToInclusive;
use core::sync::atomic::Ordering;

use ribbit::Atomic;

use crate::raw;
use crate::raw::edge;
use crate::raw::edge::Key as _;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Read as _;
use crate::raw::node::Lower as _;
use crate::raw::node::Upper as _;
use crate::raw::Edge;
use crate::raw::Key;

pub(crate) enum RangeIter<'k, 'g, const REVERSE: bool, K: Key, R: Range<'k, K>, W: key::Write> {
    Root { writer: W, next: Option<u64> },
    Node(NodeIter<'k, 'g, REVERSE, K, R, W>),
}

impl<'k, 'g, const REVERSE: bool, K, R, W> Default for RangeIter<'k, 'g, REVERSE, K, R, W>
where
    K: Key,
    R: Range<'k, K>,
    W: key::Write + From<K::Read<'k>>,
{
    fn default() -> Self {
        Self::Root {
            writer: W::default(),
            next: None,
        }
    }
}

impl<'k, 'g, const REVERSE: bool, K, R, W> RangeIter<'k, 'g, REVERSE, K, R, W>
where
    K: Key,
    R: Range<'k, K>,
    W: key::Write<Edge = K::Edge> + From<K::Read<'k>>,
{
    pub(crate) unsafe fn new_unchecked(
        root: &'g Atomic<Edge<K::Edge>>,
        prefix: K::Read<'k>,
        range: R,
    ) -> Self {
        let edge = root.load_packed(Ordering::Acquire);

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
                next: Some(value),
            },
            edge::Child::Node(node) => {
                let mut stack = Vec::with_capacity(7);
                stack.push((len, unsafe { node.entries(lower_byte, upper_byte) }));

                Self::Node(NodeIter {
                    lower,
                    upper,
                    stack,
                    writer,
                })
            }
        }
    }

    #[inline]
    pub(crate) fn for_each<F: FnMut(&W, u64) -> ControlFlow<()>>(self, mut apply: F) {
        match self {
            RangeIter::Root { writer, mut next } => {
                crate::cold();
                if let Some(value) = next.take() {
                    let _ = apply(&writer, value);
                }
            }
            RangeIter::Node(mut iter) => iter.for_each(apply),
        }
    }

    #[inline]
    pub(crate) fn lend(&mut self) -> Option<(&W, u64)> {
        match self {
            RangeIter::Root { writer: key, next } => {
                crate::cold();
                let value = next.take()?;
                Some((key, value))
            }
            RangeIter::Node(iter) => iter.lend(),
        }
    }
}

pub(crate) struct NodeIter<'k, 'g, const REVERSE: bool, K: Key, R: Range<'k, K>, W: key::Write> {
    lower: R::Lower,
    upper: R::Upper,
    writer: W,
    stack: Vec<(
        W::Len,
        raw::node::NodeIter<
            'g,
            <R::Lower as Lower<K::Read<'k>>>::Bound,
            <R::Upper as Upper<K::Read<'k>>>::Bound,
            K::Edge,
        >,
    )>,
}

impl<'k, 'g, const REVERSE: bool, K, R, W> NodeIter<'k, 'g, REVERSE, K, R, W>
where
    K: Key,
    R: Range<'k, K>,
    W: key::Write<Edge = K::Edge>,
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
            let (len, iter) = self.stack.last_mut()?;
            let len = *len;

            'horizontal: loop {
                let Some((mut byte, mut edge)) = (if REVERSE {
                    iter.next_back()
                } else {
                    iter.next()
                }) else {
                    self.stack.pop();
                    continue 'vertical;
                };

                let mut len = len;
                let mut check_lower = iter.lower().check(byte);
                let mut check_upper = iter.upper().check(byte);

                'compress: loop {
                    let (meta, child) = {
                        let edge = edge.load_packed(Ordering::Acquire);
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
                            None if REVERSE => {
                                self.stack.clear();
                                return None;
                            }
                            None => continue 'horizontal,
                        }
                    } else {
                        Default::default()
                    };

                    let upper = if check_upper {
                        match self.upper.check(meta) {
                            Some(upper) => upper,
                            None if REVERSE => continue 'horizontal,
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
                            return Some((&self.writer, value));
                        }
                        edge::Child::Value(value) => match apply(&self.writer, value) {
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

#[derive(Copy, Clone, Default)]
pub struct Unbound;

pub trait Range<'k, K: Key>: Clone {
    #[expect(private_bounds)]
    type Lower: Lower<K::Read<'k>>;

    #[expect(private_bounds)]
    type Upper: Upper<K::Read<'k>>;

    fn lower(&self, bits: usize) -> Self::Lower;
    fn upper(&self, bits: usize) -> Self::Upper;

    #[inline]
    fn common_prefix(&self) -> K::Read<'k> {
        K::Read::default()
    }
}

impl<'k, K: Key> Range<'k, K> for RangeInclusive<K::Borrow<'k>> {
    type Lower = Include<K::Read<'k>>;
    type Upper = Include<K::Read<'k>>;

    #[inline]
    fn lower(&self, bits: usize) -> Self::Lower {
        Include(K::Read::from(*self.start()).suffix(bits))
    }

    #[inline]
    fn upper(&self, bits: usize) -> Self::Upper {
        Include(K::Read::from(*self.end()).suffix(bits))
    }

    #[inline]
    fn common_prefix(&self) -> <K as Key>::Read<'k> {
        K::Read::from(*self.start()).common_prefix(K::Read::from(*self.end()))
    }
}

impl<'k, K: Key> Range<'k, K> for RangeFrom<K::Borrow<'k>> {
    type Lower = Include<K::Read<'k>>;
    type Upper = Unbound;

    #[inline]
    fn lower(&self, bits: usize) -> Self::Lower {
        Include(K::Read::from(self.start).suffix(bits))
    }

    #[inline]
    fn upper(&self, _bits: usize) -> Self::Upper {
        Unbound
    }
}

impl<'k, K: Key> Range<'k, K> for RangeToInclusive<K::Borrow<'k>> {
    type Lower = Unbound;
    type Upper = Include<K::Read<'k>>;

    #[inline]
    fn lower(&self, _bits: usize) -> Self::Lower {
        Unbound
    }

    #[inline]
    fn upper(&self, bits: usize) -> Self::Upper {
        Include(K::Read::from(self.end).suffix(bits))
    }
}

impl<'k, K: Key> Range<'k, K> for RangeFull {
    type Lower = Unbound;
    type Upper = Unbound;

    #[inline]
    fn lower(&self, _bits: usize) -> Self::Lower {
        Unbound
    }

    #[inline]
    fn upper(&self, _bits: usize) -> Self::Upper {
        Unbound
    }
}

trait Lower<R: key::Read> {
    type Bound: raw::node::Lower;

    fn check(&mut self, edge: ribbit::Packed<R::Edge>) -> Option<Self::Bound>;
}

trait Upper<R: key::Read> {
    type Bound: raw::node::Upper;

    fn check(&mut self, edge: ribbit::Packed<R::Edge>) -> Option<Self::Bound>;
}

impl<R: key::Read> Lower<R> for Include<R> {
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

impl<R: key::Read> Upper<R> for Include<R> {
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

impl<R: key::Read> Lower<R> for Unbound {
    type Bound = Unbound;

    #[inline]
    fn check(&mut self, _edge: ribbit::Packed<R::Edge>) -> Option<Self::Bound> {
        Some(Unbound)
    }
}

impl<R: key::Read> Upper<R> for Unbound {
    type Bound = Unbound;

    #[inline]
    fn check(&mut self, _edge: ribbit::Packed<R::Edge>) -> Option<Self::Bound> {
        Some(Unbound)
    }
}
