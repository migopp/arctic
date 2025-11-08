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

    unsafe fn prefix<'g, V>(node: node::Ref<'g, V>) -> Self::PrefixIter<'g, V>;
}

impl Order for Sorted {
    type PrefixIter<'g, V>
        = node::SortedIter<'g, crate::iter::Unbound, crate::iter::Unbound, V>
    where
        V: 'g;

    #[inline]
    unsafe fn prefix<'g, V>(node: node::Ref<'g, V>) -> Self::PrefixIter<'g, V> {
        node.iter_sorted()
    }
}

impl Order for core::iter::Rev<Sorted> {
    const REVERSE: bool = true;

    type PrefixIter<'g, V>
        = core::iter::Rev<node::SortedIter<'g, crate::iter::Unbound, crate::iter::Unbound, V>>
    where
        V: 'g;

    #[inline]
    unsafe fn prefix<'g, V>(node: node::Ref<'g, V>) -> Self::PrefixIter<'g, V> {
        node.iter_sorted().rev()
    }
}

impl Order for Unsorted {
    type PrefixIter<'g, V>
        = node::UnsortedIter<'g, V>
    where
        V: 'g;

    #[inline]
    unsafe fn prefix<'g, V>(node: node::Ref<'g, V>) -> Self::PrefixIter<'g, V> {
        node.iter_unsorted()
    }
}
