use core::fmt::Debug;
use core::marker::PhantomData;

use ribbit::u120;
use ribbit::u24;

mod linear;
mod node15;
mod node256;
mod node3;

use linear::Linear;
pub(crate) use node15::Node15;
pub(crate) use node256::Node256;
pub(crate) use node3::Node3;

use crate::edge;
use crate::Edge;

pub(crate) trait Node {
    fn get(&self, key: u8) -> Option<&Edge>;

    fn get_or_reserve(&self, key: u8) -> Result<&Edge, Frozen>;

    fn reserve(&mut self, key: u8) -> Option<&mut Edge>;

    fn is_frozen(&self) -> bool;

    fn freeze(&self);

    fn replace(&self, meta: &edge::Meta) -> (Op, edge::Meta, edge::Data);
}

pub(crate) trait Info: Node + Default {
    const KIND: Kind;
    const GROW: usize;

    type Grow: Info;
    type Shrink: Info;
}

#[derive(Debug)]
pub(crate) struct Frozen;

#[derive(Debug)]
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

#[derive(Clone)]
pub(crate) enum Ref<'a> {
    Node3(*mut Node3, PhantomData<&'a ()>),
    Node15(*mut Node15, PhantomData<&'a ()>),
    Node256(*mut Node256, PhantomData<&'a ()>),
}

impl<'a> Ref<'a> {
    pub(crate) unsafe fn iter(&self) -> Iter<'a> {
        match self {
            Ref::Node3(node, _) => unsafe { node.as_ref().unwrap() }.into_iter(),
            Ref::Node15(node, _) => unsafe { node.as_ref().unwrap() }.into_iter(),
            Ref::Node256(node, _) => unsafe { node.as_ref().unwrap() }.into_iter(),
        }
    }
}

impl<'a> Ref<'a> {
    #[inline]
    pub(crate) fn get(&self, key: u8) -> Option<&'a Edge> {
        match self {
            Ref::Node3(node, _) => unsafe { node.as_ref().unwrap() }.get(key),
            Ref::Node15(node, _) => unsafe { node.as_ref().unwrap() }.get(key),
            Ref::Node256(node, _) => unsafe { node.as_ref().unwrap() }.get(key),
        }
    }

    #[inline]
    pub(crate) fn get_or_reserve(&self, key: u8) -> Result<&'a Edge, Frozen> {
        match self {
            Ref::Node3(node, _) => unsafe { node.as_ref().unwrap() }.get_or_reserve(key),
            Ref::Node15(node, _) => unsafe { node.as_ref().unwrap() }.get_or_reserve(key),
            Ref::Node256(node, _) => unsafe { node.as_ref().unwrap() }.get_or_reserve(key),
        }
    }

    #[inline]
    pub(crate) fn reserve(&mut self, key: u8) -> Option<&'a mut Edge> {
        match self {
            Ref::Node3(node, _) => unsafe { node.as_mut().unwrap() }.reserve(key),
            Ref::Node15(node, _) => unsafe { node.as_mut().unwrap() }.reserve(key),
            Ref::Node256(node, _) => unsafe { node.as_mut().unwrap() }.reserve(key),
        }
    }

    #[inline]
    pub(crate) fn is_frozen(&self) -> bool {
        match self {
            Ref::Node3(node, _) => unsafe { node.as_ref().unwrap() }.is_frozen(),
            Ref::Node15(node, _) => unsafe { node.as_ref().unwrap() }.is_frozen(),
            Ref::Node256(node, _) => unsafe { node.as_ref().unwrap() }.is_frozen(),
        }
    }

    #[inline]
    pub(crate) fn freeze(&self) {
        match self {
            Ref::Node3(node, _) => unsafe { node.as_ref().unwrap() }.freeze(),
            Ref::Node15(node, _) => unsafe { node.as_ref().unwrap() }.freeze(),
            Ref::Node256(node, _) => unsafe { node.as_ref().unwrap() }.freeze(),
        }
    }

    #[inline]
    pub(crate) fn replace(&self, meta: &edge::Meta) -> (Op, edge::Meta, edge::Data) {
        match self {
            Ref::Node3(node, _) => unsafe { node.as_ref().unwrap() }.replace(meta),
            Ref::Node15(node, _) => unsafe { node.as_ref().unwrap() }.replace(meta),
            Ref::Node256(node, _) => unsafe { node.as_ref().unwrap() }.replace(meta),
        }
    }
}

impl Debug for Ref<'_> {
    fn fmt(&self, fmt: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Ref::Node3(node3, _) => unsafe { node3.as_ref().unwrap() }.fmt(fmt),
            Ref::Node15(node15, _) => unsafe { node15.as_ref().unwrap() }.fmt(fmt),
            Ref::Node256(node256, _) => unsafe { node256.as_ref().unwrap() }.fmt(fmt),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[ribbit::pack(size = 3, debug)]
pub(crate) enum Kind {
    #[ribbit(size = 0)]
    Uninit,
    #[ribbit(size = 0)]
    Removed,
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
        Self::Uninit
    }
}

pub(crate) type Iter<'a> = core::iter::Zip<KeyIter, EdgeIter<'a>>;

pub(crate) struct EdgeIter<'a> {
    edges: &'a [Edge],
    next: usize,
}

impl<'a> EdgeIter<'a> {
    pub(crate) fn new(edges: &'a [Edge]) -> Self {
        Self { edges, next: 0 }
    }
}

impl<'a> Iterator for EdgeIter<'a> {
    type Item = &'a Edge;
    fn next(&mut self) -> Option<Self::Item> {
        let edge = self.edges.get(self.next)?;
        self.next += 1;
        Some(edge)
    }
}

pub(crate) enum KeyIter {
    K3 { keys: [u8; 3], next: u8 },
    K15 { keys: [u8; 15], next: u8 },
    K256 { next: u16 },
}

impl KeyIter {
    pub(crate) fn new_3(keys: u24) -> Self {
        let keys = keys.value();
        Self::K3 {
            keys: core::array::from_fn(|index| (keys >> (index * 8)) as u8),
            next: 0,
        }
    }

    pub(crate) fn new_15(keys: u120) -> Self {
        let keys = keys.value();
        Self::K15 {
            keys: core::array::from_fn(|index| (keys >> (index * 8)) as u8),
            next: 0,
        }
    }

    pub(crate) fn new_256() -> Self {
        Self::K256 { next: 0 }
    }
}

impl Iterator for KeyIter {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            KeyIter::K3 { keys, next } => {
                let key = keys.get(*next as usize)?;
                *next += 1;
                Some(*key)
            }

            KeyIter::K15 { keys, next } => {
                let key = keys.get(*next as usize)?;
                *next += 1;
                Some(*key)
            }

            KeyIter::K256 { next } if *next >= 256 => None,
            KeyIter::K256 { next } => {
                let key = *next;
                *next += 1;
                Some(key as u8)
            }
        }
    }
}
