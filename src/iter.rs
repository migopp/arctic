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
    type Iter<'a> = node::SortedIter<'a>;
    unsafe fn new<'a>(node: node::Ref<'a>) -> Self::Iter<'a> {
        node.iter_sorted()
    }
}

impl SortPrivate for Unsorted {
    type Iter<'a> = node::UnsortedIter<'a>;
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
}
