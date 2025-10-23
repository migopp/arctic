use ribbit::atomic::Atomic128;

use crate::node;
use crate::Edge;

#[expect(private_bounds)]
pub trait Sort: SortPrivate {}

impl<T: SortPrivate> Sort for T {}

#[derive(Clone)]
pub struct Sorted;

#[derive(Clone)]
pub struct Unsorted;

pub(crate) trait SortPrivate: Clone {
    type Iter<'g, V>: Iterator<Item = (u8, &'g Atomic128<Edge<V>>)>
    where
        V: 'g;
    unsafe fn new<'g, V>(node: node::Ref<'g, V>) -> Self::Iter<'g, V>;
}

impl SortPrivate for Sorted {
    type Iter<'g, V>
        = node::SortedIter<'g, V>
    where
        V: 'g;

    #[inline]
    unsafe fn new<'g, V>(node: node::Ref<'g, V>) -> Self::Iter<'g, V> {
        node.iter_sorted()
    }
}

impl SortPrivate for core::iter::Rev<Sorted> {
    type Iter<'g, V>
        = core::iter::Rev<node::SortedIter<'g, V>>
    where
        V: 'g;

    #[inline]
    unsafe fn new<'g, V>(node: node::Ref<'g, V>) -> Self::Iter<'g, V> {
        node.iter_sorted().rev()
    }
}

impl SortPrivate for Unsorted {
    type Iter<'g, V>
        = node::UnsortedIter<'g, V>
    where
        V: 'g;

    #[inline]
    unsafe fn new<'g, V>(node: node::Ref<'g, V>) -> Self::Iter<'g, V> {
        node.iter_unsorted()
    }
}
