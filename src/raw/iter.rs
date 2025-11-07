mod postorder;
mod prefix;
mod range;
pub(crate) mod sort;

pub(crate) use postorder::PostorderIter;
pub(crate) use prefix::PrefixIter;
pub(crate) use range::RangeIter;
use ribbit::atomic::Atomic128;
pub(crate) use sort::Order;

use crate::key;
use crate::raw::Edge;

/// Abstraction over prefix and range iteration
pub(crate) trait Scan {
    type Iter<'g, R, W, C, O>: ScanIter<'g, R, W, C, O>
    where
        R: key::Read,
        W: key::Write + From<R>,
        C: 'g,
        O: Order;

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
        O: Order;
}

pub(crate) trait ScanIter<'g, R, W, C, O>: Iterator<Item = (W, u64)> {
    fn lend(&mut self) -> Option<(&W, u64)>;

    fn for_each<F: FnMut(&W, u64)>(self, apply: F);
}

pub(crate) struct Prefix;

impl Scan for Prefix {
    type Iter<'g, R, W, C, O>
        = PrefixIter<'g, W, C, O>
    where
        R: key::Read,
        W: key::Write + From<R>,
        C: 'g,
        O: Order;

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
        O: Order,
    {
        Self::Iter::new_unchecked(root, prefix)
    }
}

impl<'g, R, W, C, O> ScanIter<'g, R, W, C, O> for PrefixIter<'g, W, C, O>
where
    R: key::Read,
    W: key::Write + From<R>,
    O: Order,
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
    O: Order,
{
    type Item = (W, u64);
    fn next(&mut self) -> Option<Self::Item> {
        self.lend().map(|(key, value)| (key.clone(), value))
    }
}

pub(crate) struct Range;

impl Scan for Range {
    type Iter<'g, R, W, C, O>
        = RangeIter<'g, R, W, C, O>
    where
        R: key::Read,
        W: key::Write + From<R>,
        C: 'g,
        O: Order;

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
        O: Order,
    {
        Self::Iter::new_unchecked(root, prefix, min, max)
    }
}

impl<'g, R, W, C, O> ScanIter<'g, R, W, C, O> for RangeIter<'g, R, W, C, O>
where
    R: key::Read,
    W: key::Write + From<R>,
    O: Order,
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

impl<'g, R, W, C, O> Iterator for RangeIter<'g, R, W, C, O>
where
    R: key::Read,
    W: key::Write + From<R>,
    O: Order,
{
    type Item = (W, u64);
    fn next(&mut self) -> Option<Self::Item> {
        self.lend().map(|(key, value)| (key.clone(), value))
    }
}
