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
use crate::Edge;
use crate::Or;

pub(crate) trait Node {
    fn get(&self, key: u8) -> Option<&Atomic128<Edge>>;

    fn get_or_reserve(&self, key: u8) -> Option<&Atomic128<Edge>>;

    fn reserve(&mut self, key: u8) -> Option<&mut Atomic128<Edge>>;

    fn replace(&self, parent: ribbit::Packed<edge::Meta>) -> (Op, ribbit::Packed<Edge>);
}

pub(crate) trait Info: Node + Default {
    const KIND: ribbit::Packed<Kind>;
    const META: ribbit::Packed<edge::Meta> = edge::Meta::DEFAULT.with_kind(Self::KIND);
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
    pub(crate) unsafe fn iter_sorted(&self) -> SortedIter<'a> {
        match self {
            Ref::Node3(node) => Or::L(node.iter_sorted()),
            Ref::Node15(node) => Or::L(node.iter_sorted()),
            Ref::Node256(node) => Or::R(node.into_iter()),
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

    pub(crate) fn as_u64(&self) -> u64 {
        match *self {
            Ref::Node3(node) => node as *const _ as u64,
            Ref::Node15(node) => node as *const _ as u64,
            Ref::Node256(node) => node as *const _ as u64,
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

#[derive(Copy, Clone, Debug, PartialEq, Eq, ribbit::Pack)]
#[ribbit(size = 3, debug, eq, ord)]
pub(crate) enum Kind {
    #[ribbit(size = 0)]
    None = 0,
    #[ribbit(size = 0)]
    Leaf = 1,
    #[ribbit(size = 0)]
    Node3 = 2,
    #[ribbit(size = 0)]
    Node15 = 3,
    #[ribbit(size = 0)]
    Node256 = 4,
}

impl Kind {
    pub(crate) const NONE: ribbit::Packed<Self> = ribbit::Packed::<Self>::new_none();
    pub(crate) const LEAF: ribbit::Packed<Self> = ribbit::Packed::<Self>::new_leaf();
    pub(crate) const NODE_3: ribbit::Packed<Self> = ribbit::Packed::<Self>::new_node3();
    pub(crate) const NODE_15: ribbit::Packed<Self> = ribbit::Packed::<Self>::new_node15();
    pub(crate) const NODE_256: ribbit::Packed<Self> = ribbit::Packed::<Self>::new_node256();
}

impl Default for Kind {
    fn default() -> Self {
        Self::None
    }
}

pub(crate) type SortedIter<'a> = Or<linear::SortedIter<'a>, node256::Iter<'a>>;
pub(crate) type UnsortedIter<'a> = Or<linear::UnsortedIter<'a>, node256::Iter<'a>>;
