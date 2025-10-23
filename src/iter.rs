mod leaf;
pub(crate) mod postorder;
mod range;
mod sort;

pub(crate) use leaf::LeafIter;
pub(crate) use postorder::PostorderIter;
pub(crate) use range::RangeIter;
pub use sort::Sort;
pub use sort::Sorted;
pub use sort::Unsorted;

#[derive(Debug)]
pub(crate) enum Or<L, R> {
    L(L),
    R(R),
}

impl<L, R, T> Iterator for Or<L, R>
where
    L: Iterator<Item = T>,
    R: Iterator<Item = T>,
{
    type Item = T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Or::L(left) => left.next(),
            Or::R(right) => right.next(),
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Or::L(left) => left.size_hint(),
            Or::R(right) => right.size_hint(),
        }
    }
}

impl<L, R, T> ExactSizeIterator for Or<L, R>
where
    L: ExactSizeIterator<Item = T>,
    R: ExactSizeIterator<Item = T>,
{
    #[inline]
    fn len(&self) -> usize {
        match self {
            Or::L(left) => left.len(),
            Or::R(right) => right.len(),
        }
    }
}

impl<L, R, T> DoubleEndedIterator for Or<L, R>
where
    L: DoubleEndedIterator<Item = T>,
    R: DoubleEndedIterator<Item = T>,
{
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        match self {
            Or::L(left) => left.next_back(),
            Or::R(right) => right.next_back(),
        }
    }
}
