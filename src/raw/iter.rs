mod postorder;
mod prefix;
mod range;
pub(crate) mod sort;

pub(crate) use postorder::PostorderIter;
pub(crate) use prefix::PrefixIter;
pub(crate) use range::RangeIter;
use ribbit::atomic::Atomic128;
pub(crate) use sort::Sort;

use crate::key;
use crate::raw::Edge;

/// Abstraction over prefix and range iteration
pub(crate) trait ScanIter<'g, R, W, C, S> {
    type Input;

    unsafe fn new_unchecked(root: &'g Atomic128<Edge<C>>, prefix: R, input: Self::Input) -> Self;

    fn lend(&mut self) -> Option<(&W, u64)>;

    fn for_each<F: FnMut(&W, u64)>(self, apply: F);
}

impl<'g, R, W, C, S> ScanIter<'g, R, W, C, S> for PrefixIter<'g, W, C, S>
where
    R: key::Read,
    W: key::Write + From<R>,
    S: Sort,
{
    type Input = ();

    #[inline]
    unsafe fn new_unchecked(root: &'g Atomic128<Edge<C>>, prefix: R, (): ()) -> Self {
        Self::new_unchecked(root, prefix)
    }

    #[inline]
    fn lend(&mut self) -> Option<(&W, u64)> {
        Self::lend(self)
    }

    #[inline]
    fn for_each<F: FnMut(&W, u64)>(self, apply: F) {
        Self::for_each(self, apply)
    }
}

impl<'g, R, W, C, S> ScanIter<'g, R, W, C, S> for RangeIter<'g, R, W, C, S>
where
    R: key::Read,
    W: key::Write + From<R>,
    S: Sort,
{
    type Input = (R, R);

    #[inline]
    unsafe fn new_unchecked(root: &'g Atomic128<Edge<C>>, prefix: R, (min, max): (R, R)) -> Self {
        Self::new_unchecked(root, prefix, min, max)
    }

    #[inline]
    fn lend(&mut self) -> Option<(&W, u64)> {
        Self::lend(self)
    }

    #[inline]
    fn for_each<F: FnMut(&W, u64)>(self, apply: F) {
        Self::for_each(self, apply)
    }
}
