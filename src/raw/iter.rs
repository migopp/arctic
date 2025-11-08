mod postorder;
mod range;

use core::cmp;
use core::ops::RangeInclusive;

pub(crate) use postorder::PostorderIter;
pub(crate) use range::RangeIter;

use crate::byte;
use crate::key;
use crate::raw;

#[derive(Copy, Clone)]
pub(crate) struct Include<T>(pub(crate) T);

#[derive(Copy, Clone)]
pub(crate) struct Exclude<T>(pub(crate) T);

#[derive(Copy, Clone, Default)]
pub(crate) struct Unbound;

pub(crate) trait Range<R>: Clone {
    type Lower: Lower<R>;
    type Upper: Upper<R>;

    fn skip(self, bits: usize) -> Self;

    fn lower(&self) -> Self::Lower;
    fn upper(&self) -> Self::Upper;
}

impl<R: key::Read> Range<R> for RangeInclusive<R> {
    type Lower = Include<R>;
    type Upper = Include<R>;

    #[inline]
    fn skip(self, bits: usize) -> Self {
        let mut low = *self.start();
        let mut high = *self.end();
        low.seek(bits);
        high.seek(bits);
        low..=high
    }

    fn lower(&self) -> Self::Lower {
        Include(*self.start())
    }

    fn upper(&self) -> Self::Upper {
        Include(*self.end())
    }
}

pub(crate) trait Lower<R> {
    type Bound: raw::node::Lower;

    fn check_value(&mut self, edge: byte::Array) -> bool;

    fn check_node(&mut self, edge: byte::Array) -> Option<Self::Bound>;
}

pub(crate) trait Upper<R> {
    type Bound: raw::node::Upper;

    fn check_value(&mut self, edge: byte::Array) -> bool;

    fn check_node(&mut self, edge: byte::Array) -> Option<Self::Bound>;
}

impl<R: key::Read> Lower<R> for Include<R> {
    type Bound = Option<u8>;

    #[inline]
    fn check_value(&mut self, edge: byte::Array) -> bool {
        if self.0.bits() < edge.len().bits() as usize {
            return false;
        }

        let len = edge.len().min_bits(self.0.bits());
        self.0.take(len) >= edge
    }

    fn check_node(&mut self, edge: byte::Array) -> Option<Self::Bound> {
        let len = edge.len().min_bits(self.0.bits());
        match edge.cmp(&self.0.take(len)) {
            cmp::Ordering::Less => None,
            cmp::Ordering::Equal => Some(self.0.next()),
            cmp::Ordering::Greater => Some(Default::default()),
        }
    }
}

impl<R: key::Read> Upper<R> for Include<R> {
    type Bound = Option<u8>;

    fn check_value(&mut self, edge: byte::Array) -> bool {
        if self.0.bits() > edge.len().bits() as usize {
            return false;
        }

        let len = edge.len().min_bits(self.0.bits());
        self.0.take(len) <= edge
    }

    fn check_node(&mut self, edge: byte::Array) -> Option<Self::Bound> {
        let len = edge.len().min_bits(self.0.bits());
        match edge.cmp(&self.0.take(len)) {
            cmp::Ordering::Less => Some(Default::default()),
            cmp::Ordering::Equal => Some(self.0.next()),
            cmp::Ordering::Greater => None,
        }
    }
}

impl<R> Range<R> for core::ops::RangeFull {
    type Lower = Unbound;
    type Upper = Unbound;

    #[inline]
    fn skip(self, _bits: usize) -> Self {
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

impl<R> Lower<R> for Unbound {
    type Bound = Unbound;

    #[inline]
    fn check_value(&mut self, _edge: byte::Array) -> bool {
        true
    }

    #[inline]
    fn check_node(&mut self, _edge: byte::Array) -> Option<Self::Bound> {
        Some(Unbound)
    }
}

impl<R> Upper<R> for Unbound {
    type Bound = Unbound;

    #[inline]
    fn check_value(&mut self, _edge: byte::Array) -> bool {
        true
    }

    #[inline]
    fn check_node(&mut self, _edge: byte::Array) -> Option<Self::Bound> {
        Some(Unbound)
    }
}
