use core::fmt::Debug;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

mod node256;
mod node3;

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
    Node256(*mut Node256),
}

impl Ref {
    // FIXME: how to express lifetimes?
    pub(crate) unsafe fn as_node<'art>(&self) -> &'art dyn Node {
        match self {
            Ref::Node3(node) => unsafe { node.as_ref().unwrap() },
            Ref::Node256(node) => unsafe { node.as_ref().unwrap() },
        }
    }

    pub(crate) unsafe fn iter<'art>(&self) -> Iter<'art> {
        match self {
            Ref::Node3(node) => unsafe { node.as_ref().unwrap() }.into_iter(),
            Ref::Node256(node) => unsafe { node.as_ref().unwrap() }.into_iter(),
        }
    }
}

impl Debug for Ref {
    fn fmt(&self, fmt: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Ref::Node3(node3) => unsafe { node3.as_ref().unwrap() }.fmt(fmt),
            Ref::Node256(node256) => unsafe { node256.as_ref().unwrap() }.fmt(fmt),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[ribbit::pack(size = 2, debug)]
pub(crate) enum Kind {
    #[ribbit(size = 0)]
    None,
    #[ribbit(size = 0)]
    Leaf,
    #[ribbit(size = 0)]
    Node3,
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
        if self.next >= self.edges.len() {
            return None;
        }

        let next = self.next;
        let edge = self.edges[next].load(Ordering::Relaxed);

        self.next += 1;
        Some(edge)
    }
}

pub(crate) enum KeyIter {
    K0 { done: bool },
    K3 { keys: [u8; 3], next: u8 },
    K256 { next: u16 },
}

impl KeyIter {
    pub(crate) fn new_0() -> Self {
        Self::K0 { done: false }
    }

    pub(crate) fn new_3(keys: [u8; 3]) -> Self {
        Self::K3 { keys, next: 0 }
    }

    pub(crate) fn new_256() -> Self {
        Self::K256 { next: 0 }
    }
}

impl Iterator for KeyIter {
    // NOTE: `Option` here is only necessary to handle the root edge,
    // which has no incoming key. Is there a way to avoid this?
    type Item = Option<u8>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            KeyIter::K0 { done } if *done => None,
            KeyIter::K0 { done } => {
                *done = true;
                Some(None)
            }

            KeyIter::K3 { keys, next } => {
                let key = keys.get(*next as usize)?;
                *next += 1;
                Some(Some(*key))
            }

            KeyIter::K256 { next } if *next >= 256 => None,
            KeyIter::K256 { next } => {
                let key = *next;
                *next += 1;
                Some(Some(key as u8))
            }
        }
    }
}
