use core::fmt::Debug;
use core::sync::atomic::Ordering;

mod iter;
mod linear;
mod node_15;
mod node_256;
mod node_3;
mod node_47;
mod simd;

pub(crate) use iter::KeyIter;
pub(crate) use iter::Lower;
pub(crate) use iter::NodeIter;
pub(crate) use iter::Upper;
use linear::Linear;
pub(crate) use node_15::Node15;
pub(crate) use node_256::Node256;
pub(crate) use node_3::Node3;
pub(crate) use node_47::Node47;
use ribbit::Atomic;

use crate::raw::edge;
use crate::raw::edge::Meta as _;
use crate::raw::iter::Unbound;
use crate::raw::Edge;

pub(crate) trait Node<M>: Default
where
    M: ribbit::Pack<Packed: edge::Meta>,
{
    const KIND: Kind;
    const LEN: usize;

    type Grow: Node<M>;
    type Shrink: Node<M>;

    fn keys<L: iter::Lower, U: iter::Upper>(&self, lower: L, upper: U) -> KeyIter;

    fn iter<O: crate::iter::Order, L: iter::Lower, U: iter::Upper>(
        &self,
        lower: L,
        upper: U,
    ) -> NodeIter<L, U, M> {
        let mut keys = self.keys(lower, upper);
        if O::SORTED {
            keys.sort_unstable();
        }
        unsafe { NodeIter::new(lower, upper, keys, self.edges()) }
    }

    fn edges(&self) -> &[Atomic<Edge<M>>];

    fn get(&self, key: u8) -> Option<&Atomic<Edge<M>>>;

    fn get_or_insert(&self, key: u8) -> Option<&Atomic<Edge<M>>>;

    fn insert(&mut self, key: u8) -> Option<&mut Atomic<Edge<M>>>;

    fn freeze(&self);

    fn replace<const LEN_: usize>(
        &self,
        meta: ribbit::Packed<M>,
    ) -> (Smo, ribbit::Packed<Edge<M>>) {
        const {
            assert!(Self::LEN == LEN_);
        }

        // Caller must not call replace if doomed to fail CAS
        validate!(!meta.is_frozen());

        // Can only call replace on nodes
        validate!(!meta.is_value());

        let mut keys = [0u8; LEN_];
        let mut edges = [Edge::DEFAULT; LEN_];

        self.freeze();

        let len = self
            .iter::<crate::iter::Unsorted, _, _>(Unbound, Unbound)
            .map(|(key, edge)| (key, edge.load_packed(Ordering::Relaxed)))
            .filter(|(_, edge)| !edge.is_null())
            .map(|(key, edge)| {
                validate!(
                    edge.meta().is_frozen(),
                    "{} edge must be frozen before replace",
                    core::any::type_name::<Self>(),
                );
                (key, edge.unfreeze())
            })
            .zip(&mut keys)
            .zip(&mut edges)
            .map(|(((key_old, edge_old), key_new), edge_new)| {
                *key_new = key_old;
                *edge_new = edge_old;
            })
            .count();

        if len == 0 {
            return (Smo::Destroy, Edge::DEFAULT);
        } else if len == 1 {
            let key = keys[0];
            let edge = edges[0];
            if let Some(meta) = meta.compress(key, edge.meta()) {
                return (Smo::Compress, edge.with_meta(meta));
            }
        }

        let keys = keys.into_iter().take(len);
        let edges = edges.into_iter().take(len);

        if len == Self::LEN {
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
    Node47(&'g Node47<M>),
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
        match self {
            Self::Node3(node) => node.iter::<O, _, _>(lower, upper),
            Self::Node15(node) => node.iter::<O, _, _>(lower, upper),
            Self::Node47(node) => node.iter::<O, _, _>(lower, upper),
            Self::Node256(node) => node.iter::<O, _, _>(lower, upper),
        }
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
            Self::Node47(node) => node.get(key),
            Self::Node256(node) => node.get(key),
        }
    }

    #[inline]
    pub(crate) fn get_or_insert(&self, key: u8) -> Option<&'g Atomic<Edge<M>>> {
        match self {
            Ref::Node3(node) => node.get_or_insert(key),
            Ref::Node15(node) => node.get_or_insert(key),
            Ref::Node47(node) => node.get_or_insert(key),
            Ref::Node256(node) => node.get_or_insert(key),
        }
    }

    #[cold]
    pub(crate) fn replace(&self, parent: ribbit::Packed<M>) -> (Smo, ribbit::Packed<Edge<M>>) {
        match self {
            Self::Node3(node) => node.replace::<3>(parent),
            Self::Node15(node) => node.replace::<15>(parent),
            Self::Node47(node) => node.replace::<47>(parent),
            Self::Node256(node) => node.replace::<256>(parent),
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
            Self::Node47(node) => node.fmt(fmt),
            Self::Node256(node) => node.fmt(fmt),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ribbit::Pack)]
#[ribbit(size = 2, eq, debug)]
pub(crate) enum Kind {
    Node3 = 0,
    Node15 = 1,
    Node47 = 2,
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
    pub(crate) const NODE_47: ribbit::Packed<Kind> = ribbit::Packed::<Kind>::new_node47();
    pub(crate) const NODE_256: ribbit::Packed<Kind> = ribbit::Packed::<Kind>::new_node256();
}
