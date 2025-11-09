mod postorder;
mod range;

use core::cmp;
use core::ops::RangeInclusive;

pub(crate) use postorder::PostorderIter;
pub(crate) use range::RangeIter;

use crate::raw;
use crate::raw::edge::Meta as _;
use crate::raw::key;

#[derive(Copy, Clone)]
pub(crate) struct Include<T>(pub(crate) T);

#[derive(Copy, Clone)]
pub(crate) struct Exclude<T>(pub(crate) T);

#[derive(Copy, Clone, Default)]
pub(crate) struct Unbound;

pub(crate) trait Range<R: key::Read>: Clone {
    type Lower: Lower<R>;
    type Upper: Upper<R>;

    fn suffix(self, bits: usize) -> Self;

    fn lower(&self) -> Self::Lower;
    fn upper(&self) -> Self::Upper;
}

impl<R: key::Read> Range<R> for RangeInclusive<R> {
    type Lower = Include<R>;
    type Upper = Include<R>;

    #[inline]
    fn suffix(self, bits: usize) -> Self {
        let lower = *self.start();
        let upper = *self.end();
        lower.suffix(bits)..=upper.suffix(bits)
    }

    fn lower(&self) -> Self::Lower {
        Include(*self.start())
    }

    fn upper(&self) -> Self::Upper {
        Include(*self.end())
    }
}

pub(crate) trait Lower<R: key::Read> {
    type Bound: raw::node::Lower;

    fn check_value(&mut self, edge: ribbit::Packed<R::Edge>) -> bool;

    fn check_node(&mut self, edge: ribbit::Packed<R::Edge>) -> Option<Self::Bound>;
}

pub(crate) trait Upper<R: key::Read> {
    type Bound: raw::node::Upper;

    fn check_value(&mut self, edge: ribbit::Packed<R::Edge>) -> bool;

    fn check_node(&mut self, edge: ribbit::Packed<R::Edge>) -> Option<Self::Bound>;
}

impl<R: key::Read> Lower<R> for Include<R> {
    type Bound = Option<u8>;

    #[inline]
    fn check_value(&mut self, edge: ribbit::Packed<R::Edge>) -> bool {
        let len = R::Edge::len(edge);

        if self.0.bits() < R::Edge::len_to_bits(len) {
            return false;
        }

        match R::Edge::cmp(self.0.read(len), edge) {
            cmp::Ordering::Less => false,
            cmp::Ordering::Equal | cmp::Ordering::Greater => true,
        }
    }

    fn check_node(&mut self, edge: ribbit::Packed<R::Edge>) -> Option<Self::Bound> {
        match R::Edge::cmp(self.0.read(R::Edge::len(edge)), edge) {
            cmp::Ordering::Less => None,
            cmp::Ordering::Equal => Some(self.0.next()),
            cmp::Ordering::Greater => Some(Default::default()),
        }
    }
}

impl<R: key::Read> Upper<R> for Include<R> {
    type Bound = Option<u8>;

    fn check_value(&mut self, edge: ribbit::Packed<R::Edge>) -> bool {
        let len = R::Edge::len(edge);

        if self.0.bits() > R::Edge::len_to_bits(len) {
            return false;
        }

        match R::Edge::cmp(self.0.read(len), edge) {
            cmp::Ordering::Less | cmp::Ordering::Equal => true,
            cmp::Ordering::Greater => false,
        }
    }

    fn check_node(&mut self, edge: ribbit::Packed<R::Edge>) -> Option<Self::Bound> {
        match R::Edge::cmp(self.0.read(R::Edge::len(edge)), edge) {
            cmp::Ordering::Less => Some(Default::default()),
            cmp::Ordering::Equal => Some(self.0.next()),
            cmp::Ordering::Greater => None,
        }
    }
}

impl<R: key::Read> Range<R> for core::ops::RangeFull {
    type Lower = Unbound;
    type Upper = Unbound;

    #[inline]
    fn suffix(self, _bits: usize) -> Self {
        self
    }

    #[inline]
    fn lower(&self) -> Self::Lower {
        Unbound
    }

    #[inline]
    fn upper(&self) -> Self::Upper {
        Unbound
    }
}

impl<R: key::Read> Lower<R> for Unbound {
    type Bound = Unbound;

    fn check_value(&mut self, _edge: ribbit::Packed<R::Edge>) -> bool {
        true
    }

    fn check_node(&mut self, _edge: ribbit::Packed<R::Edge>) -> Option<Self::Bound> {
        Some(Unbound)
    }
}

impl<R: key::Read> Upper<R> for Unbound {
    type Bound = Unbound;

    #[inline]
    fn check_value(&mut self, _edge: ribbit::Packed<R::Edge>) -> bool {
        true
    }

    #[inline]
    fn check_node(&mut self, _edge: ribbit::Packed<R::Edge>) -> Option<Self::Bound> {
        Some(Unbound)
    }
}
