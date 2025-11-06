use crate::cursor;
use crate::Key;
use crate::Value;

#[expect(private_bounds)]
pub trait Sort: crate::raw::iter::Sort {}

impl<T: crate::raw::iter::Sort> Sort for T {}

pub use crate::raw::iter::sort::Sorted;
pub use crate::raw::iter::sort::Unsorted;

pub(crate) trait Scan {
    type Input<'l, K>
    where
        K: Key;

    fn scan<'g, 'l, K, C, V, S, F>(
        cursor: &cursor::Prefix<
            'g,
            'l,
            K::Read<'l>,
            C,
            V,
            cursor::path::Hybrid<'g, K::Read<'l>, C>,
        >,
        input: &Self::Input<'l, K>,
        apply: F,
    ) where
        K: Key,
        V: Value,
        S: Sort,
        F: FnMut(&K::Write, u64);
}

pub(crate) struct Prefix;

impl Scan for Prefix {
    type Input<'l, K>
        = ()
    where
        K: Key;

    fn scan<'g, 'l, K, C, V, S, F>(
        cursor: &cursor::Prefix<
            'g,
            'l,
            K::Read<'l>,
            C,
            V,
            cursor::path::Hybrid<'g, K::Read<'l>, C>,
        >,
        (): &(),
        apply: F,
    ) where
        K: Key,
        V: Value,
        S: Sort,
        F: FnMut(&K::Write, u64),
    {
        unsafe {
            crate::raw::iter::PrefixIter::<_, _, S>::new_unchecked(
                cursor.edge(),
                K::Write::from(cursor.prefix()),
            )
        }
        .for_each(apply)
    }
}

pub(crate) struct Range;

impl Scan for Range {
    type Input<'l, K>
        = (K::Read<'l>, K::Read<'l>)
    where
        K: Key;

    fn scan<'g, 'l, K, C, V, S, F>(
        cursor: &cursor::Prefix<
            'g,
            'l,
            K::Read<'l>,
            C,
            V,
            cursor::path::Hybrid<'g, K::Read<'l>, C>,
        >,
        (min, max): &Self::Input<'l, K>,
        apply: F,
    ) where
        K: Key,
        V: Value,
        S: Sort,
        F: FnMut(&K::Write, u64),
    {
        unsafe {
            crate::raw::iter::RangeIter::<K, _, S>::new_unchecked(
                cursor.edge(),
                K::Write::from(cursor.prefix()),
                *min,
                *max,
            )
        }
        .for_each(apply)
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
