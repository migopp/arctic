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

use crate::cursor;
use crate::raw::edge;
use crate::Key;
use crate::Value;

pub(crate) trait Scan {
    type Input<'l, K>
    where
        K: Key;

    fn scan<'g, 'l, K, V, S, F>(
        cursor: &cursor::Prefix<'g, 'l, K::Read<'l>, V, cursor::path::Hybrid<'g, K::Read<'l>, V>>,
        input: &Self::Input<'l, K>,
        apply: F,
    ) where
        K: Key,
        V: Value,
        S: Sort,
        F: FnMut(&K::Write, ribbit::Packed<edge::Value<V>>);
}

pub(crate) struct Prefix;

impl Scan for Prefix {
    type Input<'l, K>
        = ()
    where
        K: Key;

    fn scan<'g, 'l, K, V, S, F>(
        cursor: &cursor::Prefix<'g, 'l, K::Read<'l>, V, cursor::path::Hybrid<'g, K::Read<'l>, V>>,
        (): &(),
        apply: F,
    ) where
        K: Key,
        V: Value,
        S: Sort,
        F: FnMut(&K::Write, ribbit::Packed<edge::Value<V>>),
    {
        PrefixIter::<K::Write, _, S>::new(cursor).for_each(apply)
    }
}

pub(crate) struct Range;

impl Scan for Range {
    type Input<'l, K>
        = (K::Read<'l>, K::Read<'l>)
    where
        K: Key;

    fn scan<'g, 'l, K, V, S, F>(
        cursor: &cursor::Prefix<'g, 'l, K::Read<'l>, V, cursor::path::Hybrid<'g, K::Read<'l>, V>>,
        (min, max): &Self::Input<'l, K>,
        apply: F,
    ) where
        K: Key,
        V: Value,
        S: Sort,
        F: FnMut(&K::Write, ribbit::Packed<edge::Value<V>>),
    {
        RangeIter::<K, V, S>::new(cursor, *min, *max).for_each(apply)
    }
}

#[derive(Clone, Debug)]
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
