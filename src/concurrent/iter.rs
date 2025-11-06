use crate::concurrent::cursor;
use crate::concurrent::Value;
use crate::iter::Sort;
use crate::Key;

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
