pub(crate) mod postorder;
mod prefix;
mod range;
mod sort;

pub(crate) use postorder::PostorderIter;
pub(crate) use prefix::PrefixIter;
pub(crate) use range::RangeIter;
pub use sort::Sort;
pub use sort::Sorted;
pub use sort::Unsorted;

use crate::Key;

pub(crate) enum KeyValueIter<'g, 'k, K: Key, V> {
    Leaf(PrefixIter<'g, K::Write, V, crate::iter::Sorted>),
    // FIXME: take sort order in range iter?
    Range(RangeIter<'g, 'k, K, V>),
}

impl<'g, 'k, K, V> KeyValueIter<'g, 'k, K, V>
where
    K: Key,
{
    #[inline]
    pub(crate) fn for_each<F: FnMut(&K::Write, u64)>(&mut self, apply: F) {
        match self {
            KeyValueIter::Leaf(iter) => iter.for_each(apply),
            KeyValueIter::Range(iter) => iter.for_each(apply),
        }
    }
}

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
