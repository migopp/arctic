//! Unlike traditional hazard pointers, we use hazard *prefixes*,
//! which over-approxmiate a set of hazard pointers using a key prefix.
//!
//! First, note that every node and value in a trie can be associated
//! with a key prefix. For example, given the following trie:
//!
//! ```text
//!     N0 [ a | b ]
//!        /    |
//!       /     | c
//!      /      |
//!  N1 [f]  N2 [ d | e ]
//!     /        /   |
//!    /        /    | g
//!   /        /     |
//! (V0)     (V1)   (V2)
//! ```
//!
//! We have the following key prefixes:
//!
//! | Id | Type  | Prefix |
//! +----+-------|-------+
//! | N0 | Node  |       |
//! | N1 | Node  | a     |
//! | N2 | Node  | bc    |
//! | V0 | Value | af    |
//! | V1 | Value | bcd   |
//! | V2 | Value | bceg  |
//!
//! Second, note that each trie operation is also associated with
//! a key prefix. This can be a full key for point operations like
//! [`crate::concurrent::MapRef::get`], or a key prefix for prefix
//! operations like [`crate::concurrent::MapRef::prefix`].
//!
//! Then the core insight is that a trie operation will never access
//! nodes or values whose key prefixes do not overlap with its own.
//!
//! We use guard types to ensure that a hazard prefix is installed
//! for the lifetime of an operation. There are three types of guards.
//!
//! # Traversal guard
//!
//! A traversal guard is held by a cursor during traversal.
//! It protects all nodes and values with overlapping key prefixes from
//! reclamation. A traversal guard can be downgraded at runtime to
//! either a prefix guard or a leaf guard.
//!
//! In our example trie...
//!
//! ```text
//!     N0 [ a | b ]
//!        /    |
//!       /     | c
//!      /      |
//!  N1 [f]  N2 [ d | e ]
//!     /        /   |
//!    /        /    | g
//!   /        /     |
//! (V0)     (V1)   (V2)
//! ```
//!
//! A traversal guard with key prefix `bceg` would protect
//! nodes N0 + N2 and value V2 from reclamation. A traversal
//! guard with key prefix `b` would protect nodes N0 + N2
//! and values V1 + V2 from reclamation.
//!
//! # Prefix guard
//!
//! A prefix guard is held by non-linearizable iterators like
//! [`crate::concurrent::RangeIter`]. It protects all nodes
//! and values with key prefixes underneath its key prefix from
//! reclamation.
//!
//! # Value guard
//!
//! A value guard is held by point operations and linearizable
//! guards ([`crate::concurrent::LinearizableGuard`]). It protects
//! all values with key prefixes underneath its key prefix from
//! reclamation.

#[cfg(feature = "smr-hazard")]
mod membarrier;

#[cfg(feature = "smr-hazard")]
mod hazard;

#[cfg(feature = "smr-hazard")]
pub(crate) use hazard::{Global, Local, PrefixGuard, TraverseGuard, ValueGuard};

#[cfg(not(feature = "smr-hazard"))]
mod no_op;

#[cfg(not(feature = "smr-hazard"))]
pub(crate) use no_op::{Global, Guard, Local};
