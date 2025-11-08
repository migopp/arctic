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
    type Low: Low<R>;
    type High: High<R>;

    fn skip(self, bits: usize) -> Self;

    fn low(&self) -> Self::Low;
    fn high(&self) -> Self::High;
}

impl<R: key::Read> Range<R> for RangeInclusive<R> {
    type Low = Include<R>;
    type High = Include<R>;

    #[inline]
    fn skip(self, bits: usize) -> Self {
        let mut low = *self.start();
        let mut high = *self.end();
        low.seek(bits);
        high.seek(bits);
        low..=high
    }

    fn low(&self) -> Self::Low {
        Include(*self.start())
    }

    fn high(&self) -> Self::High {
        Include(*self.end())
    }
}

pub(crate) trait Low<R> {
    type Bound: raw::node::Low;

    fn check_value(&mut self, edge: byte::Array) -> bool;

    fn check_node(&mut self, edge: byte::Array) -> Option<Self::Bound>;
}

pub(crate) trait High<R> {
    type Bound: raw::node::High;

    fn check_value(&mut self, edge: byte::Array) -> bool;

    fn check_node(&mut self, edge: byte::Array) -> Option<Self::Bound>;
}

impl<R: key::Read> Low<R> for Include<R> {
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

impl<R> High<R> for Include<R>
where
    R: key::Read,
{
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
    type Low = Unbound;
    type High = Unbound;

    #[inline]
    fn skip(self, _bits: usize) -> Self {
        self
    }

    #[inline]
    fn low(&self) -> Self::Low {
        Unbound
    }

    #[inline]
    fn high(&self) -> Self::High {
        Unbound
    }
}

impl<R> Low<R> for Unbound {
    type Bound = Unbound;
    fn check_value(&mut self, _edge: byte::Array) -> bool {
        true
    }
    fn check_node(&mut self, _edge: byte::Array) -> Option<Self::Bound> {
        Some(Unbound)
    }
}

impl<R> High<R> for Unbound {
    type Bound = Unbound;
    fn check_value(&mut self, _edge: byte::Array) -> bool {
        true
    }
    fn check_node(&mut self, _edge: byte::Array) -> Option<Self::Bound> {
        Some(Unbound)
    }
}
