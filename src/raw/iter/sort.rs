use ribbit::atomic::Atomic128;

use crate::raw::node;
use crate::raw::Edge;

pub struct Sorted;

pub struct Unsorted;

pub(crate) trait Order {
    const REVERSE: bool = false;

    type PrefixIter<'g, C>: Iterator<Item = (u8, &'g Atomic128<Edge<C>>)>
    where
        C: 'g;

    type RangeIter<'g, C>: Iterator<Item = (u8, &'g Atomic128<Edge<C>>)>
    where
        C: 'g;

    unsafe fn prefix<'g, V>(node: node::Ref<'g, V>) -> Self::PrefixIter<'g, V>;

    unsafe fn iter<'g, C, L: crate::raw::node::Low, H: crate::raw::node::High>(
        node: node::Ref<'g, C>,
        min: L,
        max: H,
    ) -> Self::RangeIter<'g, C>;
}

impl Order for Sorted {
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

    unsafe fn iter<'g, C, L: crate::raw::node::Low, H: crate::raw::node::High>(
        node: node::Ref<'g, C>,
        min: L,
        max: H,
    ) -> Self::RangeIter<'g, C> {
        node.iter(min, max)
    }
}

impl Order for core::iter::Rev<Sorted> {
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

    unsafe fn iter<'g, C, L: crate::raw::node::Low, H: crate::raw::node::High>(
        _node: node::Ref<'g, C>,
        _min: L,
        _max: H,
    ) -> Self::RangeIter<'g, C> {
        todo!()
    }
}

impl Order for Unsorted {
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

    unsafe fn iter<'g, C, L: crate::raw::node::Low, H: crate::raw::node::High>(
        node: node::Ref<'g, C>,
        min: L,
        max: H,
    ) -> Self::RangeIter<'g, C> {
        node.iter(min, max)
    }
}
