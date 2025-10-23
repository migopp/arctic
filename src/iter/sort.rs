use ribbit::atomic::Atomic128;

use crate::node;
use crate::Edge;

#[expect(private_bounds)]
pub trait Sort: SortPrivate {}

impl<T: SortPrivate> Sort for T {}

pub struct Sorted;
pub struct Unsorted;

pub(crate) trait SortPrivate {
    type Iter<'a, V>: Iterator<Item = (u8, &'a Atomic128<Edge<V>>)>
    where
        V: 'a;
    unsafe fn new<'a, V>(node: node::Ref<'a, V>) -> Self::Iter<'a, V>;
}

impl SortPrivate for Sorted {
    type Iter<'a, V>
        = node::SortedIter<'a, V>
    where
        V: 'a;

    #[inline]
    unsafe fn new<'a, V>(node: node::Ref<'a, V>) -> Self::Iter<'a, V> {
        node.iter_sorted()
    }
}

impl SortPrivate for core::iter::Rev<Sorted> {
    type Iter<'a, V>
        = core::iter::Rev<node::SortedIter<'a, V>>
    where
        V: 'a;

    #[inline]
    unsafe fn new<'a, V>(node: node::Ref<'a, V>) -> Self::Iter<'a, V> {
        node.iter_sorted().rev()
    }
}

impl SortPrivate for Unsorted {
    type Iter<'a, V>
        = node::UnsortedIter<'a, V>
    where
        V: 'a;

    #[inline]
    unsafe fn new<'a, V>(node: node::Ref<'a, V>) -> Self::Iter<'a, V> {
        node.iter_unsorted()
    }
}
