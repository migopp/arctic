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

pub(crate) trait Node {
    fn get(&self, key: u8) -> Option<&Atomic128<Edge>>;

    fn get_or_reserve(&self, key: u8) -> Result<&Atomic128<Edge>, Frozen>;

    fn reserve(&mut self, key: u8) -> Option<&mut Atomic128<Edge>>;

    fn freeze(&self);

    fn replace(&self, parent: ribbit::Packed<edge::Meta>) -> (Op, ribbit::Packed<Edge>);
}

pub(crate) trait Info: Node + Default {
    const KIND: ribbit::Packed<Kind>;
    const META: ribbit::Packed<edge::Meta> = edge::Meta::DEFAULT.with_kind(Self::KIND);
    const GROW: usize;

    type Grow: Info;
    type Shrink: Info;
}

#[derive(Debug)]
pub(crate) struct Frozen;

#[derive(Copy, Clone, Debug)]
pub(crate) enum Op {
    /// Node shrink (smaller size)
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

impl<'a> PartialEq for Ref<'a> {
    fn eq(&self, other: &Self) -> bool {
        match (*self, *other) {
            (Ref::Node3(l), Ref::Node3(r)) => core::ptr::eq(l, r),
            (Ref::Node15(l), Ref::Node15(r)) => core::ptr::eq(l, r),
            (Ref::Node256(l), Ref::Node256(r)) => core::ptr::eq(l, r),
            _ => false,
        }
    }
}

impl<'a> Eq for Ref<'a> {}

impl<'a> Ref<'a> {
    pub(crate) unsafe fn iter(&self) -> Iter<'a> {
        match self {
            Ref::Node3(node) => node.into_iter(),
            Ref::Node15(node) => node.into_iter(),
            Ref::Node256(node) => node.into_iter(),
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
    pub(crate) fn get_or_reserve(&self, key: u8) -> Result<&'a Atomic128<Edge>, Frozen> {
        match self {
            Ref::Node3(node) => node.get_or_reserve(key),
            Ref::Node15(node) => node.get_or_reserve(key),
            Ref::Node256(node) => node.get_or_reserve(key),
        }
    }

    #[inline]
    pub(crate) fn freeze(&self) {
        match self {
            Ref::Node3(node) => node.freeze(),
            Ref::Node15(node) => node.freeze(),
            Ref::Node256(node) => node.freeze(),
        }
    }

    #[inline]
    pub(crate) fn replace(&self, meta: ribbit::Packed<edge::Meta>) -> (Op, ribbit::Packed<Edge>) {
        match self {
            Ref::Node3(node) => node.replace(meta),
            Ref::Node15(node) => node.replace(meta),
            Ref::Node256(node) => node.replace(meta),
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
#[ribbit::pack(size = 3, debug)]
pub(crate) enum Kind {
    #[ribbit(size = 0)]
    None,
    #[ribbit(size = 0)]
    Leaf,
    #[ribbit(size = 0)]
    Node3,
    #[ribbit(size = 0)]
    Node15,
    #[ribbit(size = 0)]
    Node256,
}

impl Default for Kind {
    fn default() -> Self {
        Self::None
    }
}

pub(crate) type EdgeIter<'a> = core::slice::Iter<'a, Atomic128<Edge>>;

pub(crate) type Iter<'a> = core::iter::Zip<KeyIter, EdgeIter<'a>>;

pub(crate) enum KeyIter {
    K3(core::iter::Take<core::array::IntoIter<u8, 4>>),
    K15(core::iter::Take<core::array::IntoIter<u8, 16>>),
    K256(core::ops::RangeInclusive<u8>),
}

impl KeyIter {
    pub(crate) fn new_3(keys: u32) -> Self {
        Self::K3(keys.to_ne_bytes().into_iter().take(3))
    }

    pub(crate) fn new_15(keys: u128) -> Self {
        Self::K15(keys.to_ne_bytes().into_iter().take(15))
    }

    pub(crate) fn new_256() -> Self {
        Self::K256(0..=255u8)
    }
}

impl Iterator for KeyIter {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            KeyIter::K3(iter) => iter.next(),
            KeyIter::K15(iter) => iter.next(),
            KeyIter::K256(iter) => iter.next(),
        }
    }
}
