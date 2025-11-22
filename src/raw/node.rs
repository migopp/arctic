use core::fmt::Debug;

mod iter;
mod linear;
mod node15;
mod node256;
mod node3;
mod node60;

pub(crate) use iter::KeyIter;
pub(crate) use iter::Lower;
pub(crate) use iter::NodeIter;
pub(crate) use iter::Upper;
use linear::Linear;
pub(crate) use node15::Node15;
pub(crate) use node256::Node256;
pub(crate) use node3::Node3;
pub(crate) use node60::Node60;
use ribbit::Atomic;

use crate::raw::edge;
use crate::raw::edge::Meta as _;
use crate::raw::Edge;

pub(crate) trait Node<M>: Default
where
    M: ribbit::Pack<Packed: edge::Meta>,
{
    const KIND: Kind;
    const GROW: usize;

    type Grow: Node<M>;
    type Shrink: Node<M>;

    // Work around not being able to use associated consts in array lengths
    type KeyBuffer: AsMut<[u8]>;
    type EdgeBuffer: AsMut<[ribbit::Packed<Edge<M>>]>;

    fn buffer() -> (Self::KeyBuffer, Self::EdgeBuffer);

    fn edges(&self) -> &[Atomic<Edge<M>>];

    fn get(&self, key: u8) -> Option<&Atomic<Edge<M>>>;

    fn get_or_insert(&self, key: u8) -> Option<&Atomic<Edge<M>>>;

    fn insert(&mut self, key: u8) -> Option<&mut Atomic<Edge<M>>>;

    fn freeze(
        &self,
    ) -> (
        impl Iterator<Item = u8>,
        impl Iterator<Item = ribbit::Packed<Edge<M>>>,
    );

    fn replace(&self, meta: ribbit::Packed<M>) -> (Smo, ribbit::Packed<Edge<M>>) {
        // Caller must not call replace if doomed to fail CAS
        validate!(!meta.is_frozen());

        // Can only call replace on nodes
        validate!(!meta.is_value());

        let mut len = 0;
        let (mut keys, mut edges) = Self::buffer();
        let keys = keys.as_mut();
        let edges = edges.as_mut();

        let (keys_frozen, edges_frozen) = self.freeze();

        keys_frozen
            .zip(edges_frozen)
            .filter(|(_, edge)| !edge.is_null())
            .map(|(key, edge)| {
                validate!(
                    edge.meta().is_frozen(),
                    "{} edge must be frozen before replace",
                    core::any::type_name::<Self>(),
                );
                (key, edge.unfreeze())
            })
            .zip(&mut *keys)
            .zip(&mut *edges)
            .for_each(|(((key_in, edge_in), key_out), edge_out)| {
                *key_out = key_in;
                *edge_out = edge_in;
                len += 1;
            });

        if len == 0 {
            return (Smo::Destroy, Edge::DEFAULT);
        } else if len == 1 {
            let key = keys[0];
            let edge = edges[0];
            if let Some(meta) = meta.compress(key, edge.meta()) {
                return (Smo::Compress, edge.with_meta(meta));
            }
        }

        let keys = keys.iter().take(len).copied();
        let edges = edges.iter().take(len).copied();

        if len == Self::GROW {
            (Smo::Grow, unsafe {
                Edge::new_node_unchecked::<Self::Grow, _, _>(meta, keys, edges)
            })
        } else {
            // Catch-all:
            (Smo::Replace, unsafe {
                Edge::new_node_unchecked::<Self, _, _>(meta, keys, edges)
            })
        }
    }
}

/// Node-related structural modification operation. Requires freezing.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Smo {
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

impl Smo {
    /// Whether this operation allocates a new node.
    #[inline]
    pub(crate) fn is_allocate(self) -> bool {
        match self {
            Self::Destroy | Self::Compress => false,
            Self::Grow | Self::Replace | Self::Shrink => true,
        }
    }
}

#[derive(Copy, Clone)]
pub(crate) enum Ref<'g, M: ribbit::Pack> {
    Node3(&'g Node3<M>),
    Node15(&'g Node15<M>),
    Node60(&'g Node60<M>),
    Node256(&'g Node256<M>),
}

impl<'g, M> Ref<'g, M>
where
    M: ribbit::Pack<Packed: edge::Meta>,
{
    #[inline]
    pub(crate) fn iter<O: crate::iter::Order, L: Lower, U: Upper>(
        &self,
        lower: L,
        upper: U,
    ) -> NodeIter<'g, L, U, M> {
        let (keys, edges) = match self {
            Self::Node3(node) => {
                let keys = if O::SORTED && L::UNBOUND && U::UNBOUND {
                    node.keys_sorted()
                } else if O::SORTED {
                    node.keys_range(lower, upper)
                } else {
                    node.keys_unsorted()
                };

                (keys, node.edges())
            }
            Self::Node15(node) => {
                let keys = if O::SORTED && L::UNBOUND && U::UNBOUND {
                    node.keys_sorted()
                } else if O::SORTED {
                    node.keys_range(lower, upper)
                } else {
                    node.keys_unsorted()
                };

                (keys, node.edges())
            }
            Self::Node60(_) => todo!(),
            Self::Node256(node) => (
                KeyIter::from_node_256(node.keys(lower, upper)),
                node.edges(),
            ),
        };

        unsafe { NodeIter::new(lower, upper, keys, edges) }
    }
}

impl<'g, M> Ref<'g, M>
where
    M: ribbit::Pack<Packed: edge::Meta>,
{
    #[inline]
    pub(crate) fn get(&self, key: u8) -> Option<&'g Atomic<Edge<M>>> {
        match self {
            Self::Node3(node) => node.get(key),
            Self::Node15(node) => node.get(key),
            Self::Node60(node) => node.get(key),
            Self::Node256(node) => node.get(key),
        }
    }

    #[inline]
    pub(crate) fn get_or_insert(&self, key: u8) -> Option<&'g Atomic<Edge<M>>> {
        match self {
            Ref::Node3(node) => node.get_or_insert(key),
            Ref::Node15(node) => node.get_or_insert(key),
            Ref::Node60(node) => node.get_or_insert(key),
            Ref::Node256(node) => node.get_or_insert(key),
        }
    }

    #[cold]
    pub(crate) fn replace(&self, parent: ribbit::Packed<M>) -> (Smo, ribbit::Packed<Edge<M>>) {
        match self {
            Self::Node3(node) => node.replace(parent),
            Self::Node15(node) => node.replace(parent),
            Self::Node60(node) => node.replace(parent),
            Self::Node256(node) => node.replace(parent),
        }
    }
}

impl<M> Debug for Ref<'_, M>
where
    M: ribbit::Pack<Packed: edge::Meta + Debug>,
{
    fn fmt(&self, fmt: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Node3(node) => node.fmt(fmt),
            Self::Node15(node) => node.fmt(fmt),
            Self::Node60(node) => node.fmt(fmt),
            Self::Node256(node) => node.fmt(fmt),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ribbit::Pack)]
#[ribbit(size = 2, eq, debug)]
pub(crate) enum Kind {
    Node3 = 0,
    Node15 = 1,
    Node60 = 2,
    Node256 = 3,
}

impl Default for Kind {
    fn default() -> Self {
        Self::Node3
    }
}

impl Kind {
    pub(crate) const NODE_3: ribbit::Packed<Kind> = ribbit::Packed::<Kind>::new_node3();
    pub(crate) const NODE_15: ribbit::Packed<Kind> = ribbit::Packed::<Kind>::new_node15();
    pub(crate) const NODE_60: ribbit::Packed<Kind> = ribbit::Packed::<Kind>::new_node60();
    pub(crate) const NODE_256: ribbit::Packed<Kind> = ribbit::Packed::<Kind>::new_node256();
}
