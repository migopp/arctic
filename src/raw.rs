//! This module contains the types defining the structure of the tree
//! ([`crate::raw::edge`] and [`crate::raw::node`]) and (b) the core
//! iteration (TODO) and traversal ([`crate::raw::cursor`]) logic.
//! It is "raw" with respect to safe memory reclamation ([`crate::smr`])
//! and value types ([`crate::value`]): users of this module must (a) ensure
//! that memory is not reclaimed while iterating or traversing the tree,
//! and (b) provide meaning to the raw u64 values.
//!
//! This separation has two benefits:
//! - Reduced compilation time and code duplication from monomorphization (esp. of value types).
//! - Reuse of iteration and traversal code between the concurrent and sequential maps.

pub(crate) mod cursor;
pub(crate) mod edge;
pub(crate) mod node;

pub(crate) use edge::Edge;
pub(crate) use node::Node;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    Node(node::Op),
    Edge(edge::Op),
}

impl Op {
    /// Whether this operation allocates a new node.
    #[inline]
    pub fn is_allocate(self) -> bool {
        match self {
            Self::Node(node) => node.is_allocate(),
            Self::Edge(edge) => edge.is_allocate(),
        }
    }

    /// Whether this operation retires an old node.
    #[inline]
    pub fn is_retire(self) -> bool {
        matches!(self, Self::Node(_))
    }
}
