use core::fmt::Debug;

mod linear;
mod node15;
mod node256;
mod node3;

use linear::Linear;
pub(crate) use node15::Node15;
pub(crate) use node256::Node256;
pub(crate) use node3::Node3;
use ribbit::atomic::Atomic128;

use crate::edge;
use crate::iter::Or;
use crate::Edge;

pub(crate) trait Node {
    fn get(&self, key: u8) -> Option<&Atomic128<Edge>>;

    fn get_or_reserve(&self, key: u8) -> Option<&Atomic128<Edge>>;

    fn reserve(&mut self, key: u8) -> Option<&mut Atomic128<Edge>>;

    fn replace(&self, parent: ribbit::Packed<edge::Meta>) -> (Op, ribbit::Packed<Edge>);
}

pub(crate) trait Info: Node + Default + core::fmt::Debug {
    const KIND: Kind;
    const GROW: usize;
    const REF: for<'a> fn(&'a Self) -> Ref<'a>;

    type Grow: Info;
    type Shrink: Info;
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    /// Node shrink (smaller size)
    #[expect(dead_code)]
    Shrink,

    /// Node replacement (same size)
    Replace,

    /// Node growth (larger size)
    Grow,

    /// Node elimination
    Destroy,

    /// Path compression
    Compress,
}

#[derive(Copy, Clone)]
pub(crate) enum Ref<'a> {
    Node3(&'a Node3),
    Node15(&'a Node15),
    Node256(&'a Node256),
}

impl<'a> Ref<'a> {
    #[inline]
    pub(crate) unsafe fn iter(&self) -> Iter<'a> {
        match self {
            Ref::Node3(node) => Or::L(node.iter()),
            Ref::Node15(node) => Or::L(node.iter()),
            Ref::Node256(node) => Or::R(node.into_iter()),
        }
    }

    #[inline]
    pub(crate) unsafe fn iter_rev(&self) -> RevIter<'a> {
        match self {
            Ref::Node3(node) => Or::L(node.iter().rev()),
            Ref::Node15(node) => Or::L(node.iter().rev()),
            Ref::Node256(node) => Or::R(node.iter_rev()),
        }
    }

    #[inline]
    pub(crate) unsafe fn iter_unsorted(&self) -> UnsortedIter<'a> {
        match self {
            Ref::Node3(node) => Or::L(node.iter_unsorted()),
            Ref::Node15(node) => Or::L(node.iter_unsorted()),
            Ref::Node256(node) => Or::R(node.into_iter()),
        }
    }

    #[inline]
    pub(crate) unsafe fn iter_range(&self, min: Option<u8>, max: Option<u8>) -> RangeIter<'a> {
        RangeIter::new(
            min,
            max,
            if min.is_none() && max.is_none() {
                self.iter()
            } else {
                let min = min.unwrap_or(0);
                let max = max.unwrap_or(255);
                match self {
                    Ref::Node3(node) => Or::L(node.iter_range(min, max)),
                    Ref::Node15(node) => Or::L(node.iter_range(min, max)),
                    Ref::Node256(node) => Or::R(node.iter_range(min, max)),
                }
            },
        )
    }
}

impl<'a> Ref<'a> {
    #[inline]
    pub(crate) fn get(&self, key: u8) -> Option<&'a Atomic128<Edge>> {
        match self {
            Ref::Node3(node) => node.get(key),
            Ref::Node15(node) => node.get(key),
            Ref::Node256(node) => node.get(key),
        }
    }

    #[inline]
    pub(crate) fn get_or_reserve(&self, key: u8) -> Option<&'a Atomic128<Edge>> {
        match self {
            Ref::Node3(node) => node.get_or_reserve(key),
            Ref::Node15(node) => node.get_or_reserve(key),
            Ref::Node256(node) => node.get_or_reserve(key),
        }
    }

    #[cold]
    pub(crate) fn replace(&self, meta: ribbit::Packed<edge::Meta>) -> (Op, ribbit::Packed<Edge>) {
        match self {
            Ref::Node3(node) => node.replace(meta),
            Ref::Node15(node) => node.replace(meta),
            Ref::Node256(node) => node.replace(meta),
        }
    }

    #[inline]
    pub(crate) fn as_data(&self) -> u64 {
        match *self {
            Ref::Node3(node) => node as *const _ as u64 | Kind::NODE_3,
            Ref::Node15(node) => node as *const _ as u64 | Kind::NODE_15,
            Ref::Node256(node) => node as *const _ as u64 | Kind::NODE_256,
        }
    }
}

impl Debug for Ref<'_> {
    fn fmt(&self, fmt: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Ref::Node3(node) => node.fmt(fmt),
            Ref::Node15(node) => node.fmt(fmt),
            Ref::Node256(node) => node.fmt(fmt),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Node3,
    Node15,
    Node256,
}

impl Kind {
    pub(crate) const NODE_3: u64 = Self::Node3 as u64;
    pub(crate) const NODE_15: u64 = Self::Node15 as u64;
    pub(crate) const NODE_256: u64 = Self::Node256 as u64;
}

pub(crate) type RevIter<'a> = Or<core::iter::Rev<linear::Iter<'a>>, node256::RevIter<'a>>;
pub(crate) type Iter<'a> = Or<linear::Iter<'a>, node256::Iter<'a>>;
pub(crate) type UnsortedIter<'a> = Or<linear::UnsortedIter<'a>, node256::Iter<'a>>;

pub(crate) struct RangeIter<'a> {
    min: Option<u8>,
    max: Option<u8>,
    iter: Iter<'a>,
}

impl<'a> RangeIter<'a> {
    #[inline]
    pub(crate) fn new(min: Option<u8>, max: Option<u8>, iter: Iter<'a>) -> Self {
        Self { min, max, iter }
    }

    #[inline]
    pub(crate) fn min(&self) -> Option<u8> {
        self.min
    }

    #[inline]
    pub(crate) fn max(&self) -> Option<u8> {
        self.max
    }
}

impl<'a> Iterator for RangeIter<'a> {
    type Item = (u8, &'a Atomic128<Edge>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }
}
