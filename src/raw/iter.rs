mod postorder;
mod prefix;
mod range;
pub(crate) mod sort;

use core::cmp;
use core::ops::RangeInclusive;

pub(crate) use postorder::PostorderIter;
pub(crate) use prefix::PrefixIter;
pub(crate) use range::RangeIter;
use ribbit::atomic::Atomic128;
pub(crate) use sort::Order;

use crate::byte;
use crate::iter::Unbound;
use crate::key;
use crate::raw;
use crate::raw::Edge;

/// Abstraction over prefix and range iteration
pub trait Scan {
    type Iter<'g, R, W, C, O>: ScanIter<'g, R, W, C, O>
    where
        R: key::Read,
        W: key::Write + From<R>,
        C: 'g,
        O: crate::iter::Order;

    type Input<'l, R>: Copy
    where
        R: Copy;

    unsafe fn new_unchecked<'g, 'l, R, W, C, O>(
        root: &'g Atomic128<Edge<C>>,
        input: Self::Input<'l, R>,
    ) -> Self::Iter<'g, R, W, C, O>
    where
        R: key::Read,
        W: key::Write + From<R>,
        C: 'g,
        O: crate::iter::Order;
}

pub trait ScanIter<'g, R, W, C, O>: Iterator<Item = (W, u64)> {
    fn lend(&mut self) -> Option<(&W, u64)>;

    fn for_each<F: FnMut(&W, u64)>(self, apply: F);
}

pub struct Prefix;

impl Scan for Prefix {
    type Iter<'g, R, W, C, O>
        = PrefixIter<'g, W, C, O>
    where
        R: key::Read,
        W: key::Write + From<R>,
        C: 'g,
        O: crate::iter::Order;

    type Input<'l, R>
        = R
    where
        R: Copy;

    #[inline]
    unsafe fn new_unchecked<'g, 'l, R, W, C, O>(
        root: &'g Atomic128<Edge<C>>,
        prefix: Self::Input<'l, R>,
    ) -> Self::Iter<'g, R, W, C, O>
    where
        R: key::Read,
        W: key::Write + From<R>,
        C: 'g,
        O: crate::iter::Order,
    {
        Self::Iter::new_unchecked(root, prefix)
    }
}

impl<'g, R, W, C, O> ScanIter<'g, R, W, C, O> for PrefixIter<'g, W, C, O>
where
    R: key::Read,
    W: key::Write + From<R>,
    O: crate::iter::Order,
{
    #[inline]
    fn lend(&mut self) -> Option<(&W, u64)> {
        Self::lend(self)
    }

    #[inline]
    fn for_each<F: FnMut(&W, u64)>(self, apply: F) {
        Self::for_each(self, apply)
    }
}

impl<'g, W, C, O> Iterator for PrefixIter<'g, W, C, O>
where
    W: key::Write,
    O: crate::iter::Order,
{
    type Item = (W, u64);
    fn next(&mut self) -> Option<Self::Item> {
        self.lend().map(|(key, value)| (key.clone(), value))
    }
}

pub struct Range;

impl Scan for Range {
    type Iter<'g, R, W, C, O>
        = RangeIter<'g, R, W, C, RangeInclusive<R>, O>
    where
        R: key::Read,
        W: key::Write + From<R>,
        C: 'g,
        O: crate::iter::Order;

    type Input<'l, R>
        = (R, R, R)
    where
        R: Copy;

    #[inline]
    unsafe fn new_unchecked<'g, 'l, R, W, C, O>(
        root: &'g Atomic128<Edge<C>>,
        (prefix, min, max): Self::Input<'l, R>,
    ) -> Self::Iter<'g, R, W, C, O>
    where
        R: key::Read,
        W: key::Write + From<R>,
        C: 'g,
        O: crate::iter::Order,
    {
        Self::Iter::new_unchecked(root, prefix, min..=max)
    }
}

impl<'g, R, W, C, B, O> ScanIter<'g, R, W, C, O> for RangeIter<'g, R, W, C, B, O>
where
    R: key::Read,
    W: key::Write + From<R>,
    B: Range_<R>,
    O: crate::iter::Order,
{
    #[inline]
    fn lend(&mut self) -> Option<(&W, u64)> {
        Self::lend(self)
    }

    #[inline]
    fn for_each<F: FnMut(&W, u64)>(self, apply: F) {
        Self::for_each(self, apply)
    }
}

impl<'g, R, W, C, B, O> Iterator for RangeIter<'g, R, W, C, B, O>
where
    R: key::Read,
    W: key::Write + From<R>,
    B: Range_<R>,
    O: crate::iter::Order,
{
    type Item = (W, u64);
    fn next(&mut self) -> Option<Self::Item> {
        self.lend().map(|(key, value)| (key.clone(), value))
    }
}

pub(crate) trait Range_<R> {
    type Low: Low<R>;
    type High: High<R>;

    fn skip(self, bits: usize) -> Self;

    fn low(&self) -> Self::Low;
    fn high(&self) -> Self::High;
}

impl<R: key::Read> Range_<R> for RangeInclusive<R> {
    type Low = crate::iter::Include<R>;
    type High = crate::iter::Include<R>;

    #[inline]
    fn skip(self, bits: usize) -> Self {
        let mut low = *self.start();
        let mut high = *self.end();
        low.seek(bits);
        high.seek(bits);
        low..=high
    }

    fn low(&self) -> Self::Low {
        crate::iter::Include(*self.start())
    }

    fn high(&self) -> Self::High {
        crate::iter::Include(*self.end())
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

impl<R: key::Read> Low<R> for crate::iter::Include<R> {
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

impl<R> High<R> for crate::iter::Include<R>
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

impl<R> Range_<R> for core::ops::RangeFull {
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
