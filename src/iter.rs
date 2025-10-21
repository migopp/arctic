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
