use ribbit::atomic::Atomic128;

use crate::node;
use crate::Edge;

#[expect(private_bounds)]
pub trait Sort: SortPrivate {}
impl<T: SortPrivate> Sort for T {}

pub struct Sorted;
pub struct Unsorted;

pub(crate) trait SortPrivate {
    type Iter<'a>: Iterator<Item = (u8, &'a Atomic128<Edge>)>;
    unsafe fn new<'a>(node: node::Ref<'a>) -> Self::Iter<'a>;
}

impl SortPrivate for Sorted {
    type Iter<'a> = node::Iter<'a>;

    #[inline]
    unsafe fn new<'a>(node: node::Ref<'a>) -> Self::Iter<'a> {
        node.iter()
    }
}

impl SortPrivate for core::iter::Rev<Sorted> {
    type Iter<'a> = node::RevIter<'a>;

    #[inline]
    unsafe fn new<'a>(node: node::Ref<'a>) -> Self::Iter<'a> {
        node.iter_rev()
    }
}

impl SortPrivate for Unsorted {
    type Iter<'a> = node::UnsortedIter<'a>;

    #[inline]
    unsafe fn new<'a>(node: node::Ref<'a>) -> Self::Iter<'a> {
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
