use ribbit::atomic::Atomic128;

use crate::raw::node;
use crate::raw::Edge;

pub struct Sorted;

pub struct Unsorted;

pub(crate) trait Sort {
    const REVERSE: bool = false;

    type PrefixIter<'g, C>: Iterator<Item = (u8, &'g Atomic128<Edge<C>>)>
    where
        C: 'g;

    type RangeIter<'g, C>: Iterator<Item = (u8, &'g Atomic128<Edge<C>>)>
    where
        C: 'g;

    unsafe fn prefix<'g, V>(node: node::Ref<'g, V>) -> Self::PrefixIter<'g, V>;

    unsafe fn range<'g, V>(
        node: node::Ref<'g, V>,
        min: Option<u8>,
        max: Option<u8>,
    ) -> Self::RangeIter<'g, V>;
}

impl Sort for Sorted {
    type PrefixIter<'g, V>
        = node::SortedIter<'g, V>
    where
        V: 'g;

    type RangeIter<'g, V>
        = node::SortedIter<'g, V>
    where
        V: 'g;

    #[inline]
    unsafe fn prefix<'g, V>(node: node::Ref<'g, V>) -> Self::PrefixIter<'g, V> {
        node.iter_sorted()
    }

    #[inline]
    unsafe fn range<'g, V>(
        node: node::Ref<'g, V>,
        min: Option<u8>,
        max: Option<u8>,
    ) -> Self::PrefixIter<'g, V> {
        node.iter_range(min, max)
    }
}

impl Sort for core::iter::Rev<Sorted> {
    const REVERSE: bool = true;

    type PrefixIter<'g, V>
        = core::iter::Rev<node::SortedIter<'g, V>>
    where
        V: 'g;

    type RangeIter<'g, V>
        = core::iter::Rev<node::SortedIter<'g, V>>
    where
        V: 'g;

    #[inline]
    unsafe fn prefix<'g, V>(node: node::Ref<'g, V>) -> Self::PrefixIter<'g, V> {
        node.iter_sorted().rev()
    }

    #[inline]
    unsafe fn range<'g, V>(
        node: node::Ref<'g, V>,
        min: Option<u8>,
        max: Option<u8>,
    ) -> Self::RangeIter<'g, V> {
        validate!(min.zip(max).map(|(min, max)| min >= max).unwrap_or(true));
        node.iter_range(max, min).rev()
    }
}

impl Sort for Unsorted {
    type PrefixIter<'g, V>
        = node::UnsortedIter<'g, V>
    where
        V: 'g;

    type RangeIter<'g, V>
        = node::SortedIter<'g, V>
    where
        V: 'g;

    #[inline]
    unsafe fn prefix<'g, V>(node: node::Ref<'g, V>) -> Self::PrefixIter<'g, V> {
        node.iter_unsorted()
    }

    #[inline]
    unsafe fn range<'g, V>(
        node: node::Ref<'g, V>,
        min: Option<u8>,
        max: Option<u8>,
    ) -> Self::RangeIter<'g, V> {
        node.iter_range(min, max)
    }
}
