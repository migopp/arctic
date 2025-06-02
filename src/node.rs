use core::fmt::Debug;

use ribbit::atomic::A128;

mod node256;
mod node3;

pub(crate) use node256::Node256;
pub(crate) use node3::Node3;

use crate::Slot;

pub(crate) trait Node {
    fn get(&self, key: u8) -> Option<&A128<Slot>>;

    fn get_or_reserve(&self, key: u8) -> Result<&A128<Slot>, GetOrReserveError>;

    fn reserve(&mut self, key: u8) -> Option<&mut A128<Slot>>;

    fn freeze(&self, grow: bool);

    fn replace(&self, snapshot: &Slot) -> (Op, Slot);
}

#[derive(Debug)]
pub(crate) enum GetOrReserveError {
    /// Encountered SMO operation in current node
    Freeze { grow: bool },

    /// Initiate grow SMO in current node
    Grow,
}

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
}

impl Debug for Ref {
    fn fmt(&self, fmt: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Ref::Node3(node3) => unsafe { node3.as_ref().unwrap() }.fmt(fmt),
            Ref::Node256(node256) => unsafe { node256.as_ref().unwrap() }.fmt(fmt),
        }
    }
}

#[derive(PartialEq, Eq)]
#[ribbit::pack(size = 3, debug)]
pub(crate) enum Kind {
    Uninit,
    Valid,
    Invalid,
    Node3,
    Node256,
}
