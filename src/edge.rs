use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::key;
use crate::node;
use crate::node::Node15;
use crate::node::Node256;
use crate::node::Node3;
use crate::Or;

#[ribbit::pack(size = 128)]
#[derive(Copy, Clone, Default, Debug)]
pub(crate) struct Edge {
    #[ribbit(size = 63)]
    pub(crate) meta: Meta,
    #[ribbit(offset = 64, size = 64)]
    pub(crate) data: Data,
}

impl Edge {
    pub(crate) fn unfreeze(&self) -> Self {
        Self {
            meta: self.meta.unfreeze(),
            data: self.data,
        }
    }

    pub(crate) fn freeze(edge: &Atomic128<Self>) {
        let mut old = edge.load_packed(Ordering::Relaxed);

        while !old.meta().frozen() {
            match edge.compare_exchange_packed(
                old,
                old.with_meta(old.meta().with_frozen(true)),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(conflict) => old = conflict,
            }
        }
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[ribbit::pack(size = 63, eq)]
pub(crate) struct Meta {
    #[ribbit(size = 59)]
    pub(crate) key: key::Array,
    pub(crate) frozen: bool,
    #[ribbit(size = 3)]
    pub(crate) kind: node::Kind,
}

impl Meta {
    fn unfreeze(&self) -> Self {
        Self {
            frozen: false,
            ..*self
        }
    }
}

#[repr(transparent)]
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
#[ribbit::pack(size = 64)]
pub(crate) struct Data(u64);

impl Data {
    pub(crate) fn new_node<N, I>(edges: I) -> Self
    where
        N: node::Info,
        I: IntoIterator<Item = (u8, Edge)>,
    {
        let mut node = Box::new(N::default());

        for (key, edge) in edges {
            node.reserve(key).expect("Node can fit all edges").set(edge);
        }

        let node = Box::leak(node) as *mut N;
        Self(node as u64)
    }

    pub(crate) fn new_leaf(leaf: u64) -> Self {
        Self(leaf)
    }

    pub(crate) fn to_leaf(self) -> u64 {
        self.0
    }

    pub(crate) unsafe fn to_node<'a>(self, kind: node::Kind) -> Option<Or<u64, node::Ref<'a>>> {
        match kind {
            node::Kind::None => None,
            node::Kind::Leaf => Some(Or::L(self.0)),
            node::Kind::Node3 => (self.0 as *mut Node3)
                .as_ref()
                .map(node::Ref::Node3)
                .map(Or::R),
            node::Kind::Node15 => (self.0 as *mut Node15)
                .as_ref()
                .map(node::Ref::Node15)
                .map(Or::R),
            node::Kind::Node256 => (self.0 as *mut Node256)
                .as_ref()
                .map(node::Ref::Node256)
                .map(Or::R),
        }
    }

    pub(crate) unsafe fn deallocate(self, kind: node::Kind) {
        match kind {
            node::Kind::None | node::Kind::Leaf => {
                unreachable!()
            }
            node::Kind::Node3 => drop(Box::from_raw(self.0 as *mut Node3)),
            node::Kind::Node15 => drop(Box::from_raw(self.0 as *mut Node15)),
            node::Kind::Node256 => drop(Box::from_raw(self.0 as *mut Node256)),
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub(crate) enum Op {
    /// Node creation
    Create,

    /// Path expansion
    Expand,

    /// Leaf insertion
    Insert,

    /// Leaf removal
    Remove,
}
