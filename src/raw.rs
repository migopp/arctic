//! This module contains the types defining the structure of the tree
//! ([`crate::raw::edge`] and [`crate::raw::node`]) and (b) the core
//! iteration ([`crate::raw::iter`]) and traversal ([`crate::raw::cursor`]) logic.
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
pub(crate) mod iter;
pub(crate) mod key;
pub(crate) mod node;

pub(crate) use cursor::Cursor;
pub(crate) use edge::Edge;
pub use key::Key;
pub(crate) use node::Node;

pub(crate) struct Frozen;

/// Structural modification operation.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Smo {
    #[expect(dead_code)]
    CompressNode,
    ReplaceNode,
    ExpandNode,
    DeleteNode,
    CompressEdge,
}

impl Smo {
    #[inline]
    pub fn is_allocate(self) -> bool {
        matches!(
            self,
            Self::CompressNode | Self::ReplaceNode | Self::ExpandNode
        )
    }
}
