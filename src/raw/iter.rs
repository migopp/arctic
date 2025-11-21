mod postorder;
mod range;

use core::cmp;
use core::ops::RangeInclusive;

pub(crate) use postorder::PostorderIter;
pub(crate) use range::RangeIter;

use crate::raw;
use crate::raw::edge::Key as _;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key;

#[derive(Copy, Clone)]
pub(crate) struct Include<T>(pub(crate) T);

#[derive(Copy, Clone)]
pub(crate) struct Exclude<T>(pub(crate) T);

#[derive(Copy, Clone, Default)]
pub(crate) struct Unbound;

pub(crate) trait Range<R: key::Read>: Clone {
    #[expect(private_bounds)]
    type Lower: Lower<R>;

    #[expect(private_bounds)]
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

        if self.0.bits() < len.bits() {
            return None;
        }

        match self.0.read(len).cmp(&key) {
            cmp::Ordering::Less => None,
            cmp::Ordering::Equal => Some(self.0.next()),
            cmp::Ordering::Greater => Some(None),
        }
    }
}

impl<R: key::Read> Upper<R> for Include<R> {
    type Bound = Option<u8>;

    fn check(&mut self, edge: ribbit::Packed<R::Edge>) -> Option<Self::Bound> {
        let key = edge.key();
        let len = key.len();

        if self.0.bits() > len.bits() {
            return None;
        }

        match self.0.read(len).cmp(&key) {
            cmp::Ordering::Less => Some(None),
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
