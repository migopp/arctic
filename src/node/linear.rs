use core::fmt::Debug;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::edge;
use crate::key;
use crate::node;
use crate::node::Edge;
use crate::node::Frozen;
use crate::node::Op;
use crate::Node;

#[repr(C, align(64))]
#[derive(Debug)]
pub(crate) struct Linear<const LEN: usize, H> {
    pub(super) header: H,
    pub(super) edges: [Atomic128<Edge>; LEN],
}

impl<const LEN: usize, H: Default> Default for Linear<LEN, H> {
    fn default() -> Self {
        Self {
            header: H::default(),
            edges: core::array::from_fn(|_| Atomic128::default()),
        }
    }
}

impl<const LEN: usize, H> Node for Linear<LEN, H>
where
    H: Header,
    Self: node::Info,
{
    fn get(&self, key: u8) -> Option<&Atomic128<Edge>> {
        let index = self.header.get(key)?;
        Some(unsafe { self.edges.get_unchecked(index as usize) })
    }

    fn get_or_reserve(&self, key: u8) -> Result<&Atomic128<Edge>, Frozen> {
        let index = self.header.get_or_reserve(key)?;
        Ok(unsafe { self.edges.get_unchecked(index as usize) })
    }

    fn reserve(&mut self, key: u8) -> Option<&mut Atomic128<Edge>> {
        match self.header.get_or_reserve(key) {
            Ok(index) => Some(unsafe { self.edges.get_unchecked_mut(index as usize) }),
            Err(_) => None,
        }
    }

    fn freeze(&self) {
        let len = self.header.freeze();
        self.edges.iter().take(len).for_each(Edge::freeze);
    }

    fn replace(&self, snapshot: &edge::Meta) -> (Op, Edge) {
        if cfg!(feature = "validate") {
            assert!(
                self.header.is_frozen(),
                "{} header must be frozen before replace",
                core::any::type_name::<Self>(),
            );
        }

        let mut edges: [(u8, Edge); LEN] = core::array::from_fn(|_| (0, Edge::default()));
        let mut len = 0;

        self.edges
            .iter()
            .map(|edge| edge.load(Ordering::Relaxed))
            .zip(self.header.keys())
            .inspect(|(edge, _)| {
                if cfg!(feature = "validate") {
                    assert!(
                        edge.meta.frozen,
                        "{} edge must be frozen before replace",
                        core::any::type_name::<Self>(),
                    )
                }
            })
            .filter(|(edge, _)| !matches!(edge.meta.kind, node::Kind::None))
            .map(|(edge, key)| {
                (
                    key,
                    Edge {
                        meta: edge::Meta {
                            frozen: false,
                            ..edge.meta
                        },
                        ..edge
                    },
                )
            })
            .zip(&mut edges)
            .for_each(|(edge, save)| {
                *save = edge;
                len += 1;
            });

        match &edges[..len] {
            [] => (
                Op::Destroy,
                Edge {
                    meta: edge::Meta {
                        key: key::Array::default(),
                        kind: node::Kind::None,
                        frozen: false,
                    },
                    data: edge::Data::default(),
                },
            ),

            [(key, edge)] if key::Array::can_compress(&snapshot.key, &edge.meta.key) => (
                Op::Compress,
                Edge {
                    meta: edge::Meta {
                        key: unsafe { key::Array::compress(&snapshot.key, *key, &edge.meta.key) },
                        kind: snapshot.kind,
                        frozen: false,
                    },
                    data: edge.data,
                },
            ),

            // Grow
            _ if len == <Self as node::Info>::GROW => (
                node::Op::Grow,
                Edge {
                    meta: edge::Meta {
                        key: snapshot.key,
                        kind: <<Self as node::Info>::Grow as node::Info>::KIND,
                        frozen: false,
                    },
                    data: edge::Data::new_node::<<Self as node::Info>::Grow, _>(
                        edges.into_iter().take(len),
                    ),
                },
            ),

            // Replace
            _ => (
                node::Op::Replace,
                Edge {
                    meta: edge::Meta {
                        key: snapshot.key,
                        kind: <Self as node::Info>::KIND,
                        frozen: false,
                    },
                    data: edge::Data::new_node::<Self, _>(edges.into_iter().take(len)),
                },
            ),
        }
    }
}

pub(super) trait Header {
    fn is_frozen(&self) -> bool;
    fn freeze(&self) -> usize;
    fn get(&self, key: u8) -> Option<u8>;
    fn get_or_reserve(&self, key: u8) -> Result<u8, Frozen>;
    fn keys(&self) -> super::KeyIter;
}
