use core::fmt::Debug;

mod iter;
mod linear;
mod node15;
mod node256;
mod node3;

pub(crate) use iter::KeyIter;
pub(crate) use iter::Lower;
pub(crate) use iter::NodeIter;
pub(crate) use iter::Upper;
use linear::Linear;
pub(crate) use node15::Node15;
pub(crate) use node256::Node256;
pub(crate) use node3::Node3;
use ribbit::Atomic;

use crate::raw::edge;
use crate::raw::Edge;

pub(crate) trait Node<M>: Default
where
    M: ribbit::Pack<Packed: edge::Meta>,
{
    const KIND: Kind;
    const GROW: usize;

    type Grow: Node<M>;
    type Shrink: Node<M>;

    fn edges(&self) -> &[Atomic<Edge<M>>];

    fn get(&self, key: u8) -> Option<&Atomic<Edge<M>>>;

    fn get_or_reserve(&self, key: u8) -> Option<&Atomic<Edge<M>>>;

    fn reserve(&mut self, key: u8) -> Option<&mut Atomic<Edge<M>>>;

    fn replace(&self, parent: ribbit::Packed<M>) -> (Op, ribbit::Packed<Edge<M>>);
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Op {
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

impl Op {
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
                    KeyIter::from_linear(node.keys_sorted())
                } else if O::SORTED {
                    KeyIter::from_linear(node.keys_range(lower, upper))
                } else {
                    KeyIter::from_linear(node.keys_unsorted())
                };

                (keys, node.edges())
            }
            Self::Node15(node) => {
                let keys = if O::SORTED && L::UNBOUND && U::UNBOUND {
                    KeyIter::from_linear(node.keys_sorted())
                } else if O::SORTED {
                    KeyIter::from_linear(node.keys_range(lower, upper))
                } else {
                    KeyIter::from_linear(node.keys_unsorted())
                };

                (keys, node.edges())
            }
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
            Self::Node256(node) => node.get(key),
        }
    }

    #[inline]
    pub(crate) fn get_or_reserve(&self, key: u8) -> Option<&'g Atomic<Edge<M>>> {
        match self {
            Ref::Node3(node) => node.get_or_reserve(key),
            Ref::Node15(node) => node.get_or_reserve(key),
            Ref::Node256(node) => node.get_or_reserve(key),
        }
    }

    #[cold]
    pub(crate) fn replace(&self, parent: ribbit::Packed<M>) -> (Op, ribbit::Packed<Edge<M>>) {
        match self {
            Self::Node3(node) => node.replace(parent),
            Self::Node15(node) => node.replace(parent),
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
            Self::Node256(node) => node.fmt(fmt),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ribbit::Pack)]
#[ribbit(size = 2, eq, debug)]
pub(crate) enum Kind {
    Node3 = 0,
    Node15 = 1,
    Node256 = 2,
}

impl Default for Kind {
    fn default() -> Self {
        Self::Node3
    }
}

impl Kind {
    pub(crate) const NODE_3: ribbit::Packed<Kind> = ribbit::Packed::<Kind>::new_node3();
    pub(crate) const NODE_15: ribbit::Packed<Kind> = ribbit::Packed::<Kind>::new_node15();
    pub(crate) const NODE_256: ribbit::Packed<Kind> = ribbit::Packed::<Kind>::new_node256();
}
