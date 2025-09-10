use core::fmt::Debug;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;
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

use crate::Edge;

pub(crate) trait Node {
    fn get(&self, key: u8) -> Option<&Atomic128<Edge>>;

    fn get_or_reserve(&self, key: u8) -> Result<&Atomic128<Edge>, Frozen>;

    fn reserve(&mut self, key: u8) -> Option<&mut Atomic128<Edge>>;

    fn is_frozen(&self) -> bool;

    fn freeze(&self);

    fn replace(&self, snapshot: &Edge) -> (Op, Edge);
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
pub(crate) enum Ref {
    Node3(*mut Node3),
    Node15(*mut Node15),
    Node256(*mut Node256),
}

impl Ref {
    // FIXME: how to express lifetimes?
    pub(crate) unsafe fn as_node<'art>(&self) -> &'art dyn Node {
        match self {
            Ref::Node3(node) => unsafe { node.as_ref().unwrap() },
            Ref::Node15(node) => unsafe { node.as_ref().unwrap() },
            Ref::Node256(node) => unsafe { node.as_ref().unwrap() },
        }
    }

    pub(crate) unsafe fn iter<'art>(&self) -> Iter<'art> {
        match self {
            Ref::Node3(node) => unsafe { node.as_ref().unwrap() }.into_iter(),
            Ref::Node15(node) => unsafe { node.as_ref().unwrap() }.into_iter(),
            Ref::Node256(node) => unsafe { node.as_ref().unwrap() }.into_iter(),
        }
    }
}

impl Debug for Ref {
    fn fmt(&self, fmt: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Ref::Node3(node3) => unsafe { node3.as_ref().unwrap() }.fmt(fmt),
            Ref::Node15(node15) => unsafe { node15.as_ref().unwrap() }.fmt(fmt),
            Ref::Node256(node256) => unsafe { node256.as_ref().unwrap() }.fmt(fmt),
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

pub(crate) type Iter<'a> = core::iter::Zip<KeyIter, EdgeIter<'a>>;

pub(crate) struct EdgeIter<'a> {
    edges: &'a [Atomic128<Edge>],
    next: usize,
}

impl<'a> EdgeIter<'a> {
    pub(crate) fn new(edges: &'a [Atomic128<Edge>]) -> Self {
        Self { edges, next: 0 }
    }
}

impl<'a> Iterator for EdgeIter<'a> {
    type Item = Edge;
    fn next(&mut self) -> Option<Self::Item> {
        let edge = self.edges.get(self.next)?.load(Ordering::Relaxed);
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
