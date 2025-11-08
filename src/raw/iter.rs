mod postorder;
mod prefix;
mod range;
pub(crate) mod sort;

use core::ops::RangeInclusive;

pub(crate) use postorder::PostorderIter;
pub(crate) use prefix::PrefixIter;
pub(crate) use range::RangeIter;
use ribbit::atomic::Atomic128;
pub(crate) use sort::Order;

use crate::byte;
use crate::key;
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
    type Min: Bound<R>;
    type Max: Bound<R>;

    fn min(&self) -> Self::Min;
    fn max(&self) -> Self::Max;
}

impl<R: key::Read> Range_<R> for RangeInclusive<R> {
    type Min = crate::iter::Include<R>;
    type Max = crate::iter::Include<R>;

    fn min(&self) -> Self::Min {
        crate::iter::Include(*self.start())
    }

    fn max(&self) -> Self::Max {
        crate::iter::Include(*self.end())
    }
}

pub(crate) trait Bound<R> {
    fn seek(&mut self, bits: usize);

    fn take_min(&mut self, len: byte::Len) -> Option<byte::Array>;
    fn take_max(&mut self, len: byte::Len) -> byte::Array;

    fn next_min(&mut self) -> Option<u8>;
    fn next_max(&mut self) -> Option<u8>;
}

impl<R> Bound<R> for crate::iter::Include<R>
where
    R: key::Read,
{
    #[inline]
    fn seek(&mut self, bits: usize) {
        self.0.seek(bits);
    }

    #[inline]
    fn take_min(&mut self, len: byte::Len) -> Option<byte::Array> {
        Some(self.0.take(len.min_bits(self.0.bits())))
    }

    #[inline]
    fn take_max(&mut self, len: byte::Len) -> byte::Array {
        self.0.take(len.min_bits(self.0.bits()))
    }

    #[inline]
    fn next_min(&mut self) -> Option<u8> {
        self.0.next()
    }

    #[inline]
    fn next_max(&mut self) -> Option<u8> {
        self.0.next()
    }
}

impl<R> Bound<R> for crate::iter::Exclude<R>
where
    R: key::Read,
{
    fn seek(&mut self, bits: usize) {
        self.0.seek(bits);
    }

    fn take_min(&mut self, len: byte::Len) -> Option<byte::Array> {
        todo!()
    }

    fn take_max(&mut self, len: byte::Len) -> byte::Array {
        todo!()
    }

    fn next_min(&mut self) -> Option<u8> {
        todo!()
    }

    fn next_max(&mut self) -> Option<u8> {
        todo!()
    }
}

impl<R> Bound<R> for crate::iter::Unbound
where
    R: key::Read,
{
    fn seek(&mut self, _bits: usize) {}

    fn take_min(&mut self, len: byte::Len) -> Option<byte::Array> {
        todo!()
    }

    fn take_max(&mut self, len: byte::Len) -> byte::Array {
        todo!()
    }

    fn next_min(&mut self) -> Option<u8> {
        todo!()
    }

    fn next_max(&mut self) -> Option<u8> {
        todo!()
    }
}
