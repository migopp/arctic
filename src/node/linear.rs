use core::fmt::Debug;
use core::sync::atomic::Ordering;

use crate::edge;
use crate::key;
use crate::node;
use crate::node::Edge;
use crate::node::Frozen;
use crate::node::Op;
use crate::Node;

#[repr(C)]
#[derive(Debug)]
pub(crate) struct Linear<const LEN: usize, H> {
    pub(super) header: H,
    pub(super) edges: [Edge; LEN],
}

impl<const LEN: usize, H: Default> Default for Linear<LEN, H> {
    fn default() -> Self {
        Self {
            header: H::default(),
            edges: core::array::from_fn(|_| Edge::default()),
        }
    }
}

impl<const LEN: usize, H> Node for Linear<LEN, H>
where
    H: Header,
    Self: node::Info,
{
    fn get(&self, key: u8) -> Option<&Edge> {
        let index = self.header.get(key);
        self.edges.get(index)
    }

    fn get_or_reserve(&self, key: u8) -> Result<&Edge, Frozen> {
        let index = self.header.get_or_reserve(key)?;
        Ok(&self.edges[index])
    }

    fn reserve(&mut self, key: u8) -> Option<&mut Edge> {
        match self.header.get_or_reserve(key) {
            Ok(index) => Some(&mut self.edges[index]),
            Err(_) => None,
        }
    }

    fn is_frozen(&self) -> bool {
        self.header.is_frozen()
    }

    fn freeze(&self) {
        let len = self.header.freeze();
        self.edges.iter().take(len).for_each(Edge::freeze);
    }

    fn replace(&self, snapshot: &edge::Meta) -> (Op, edge::Meta, edge::Data) {
        if cfg!(feature = "validate") {
            assert!(
                self.header.is_frozen(),
                "{} header must be frozen before replace",
                core::any::type_name::<Self>(),
            );
        }

        let mut edges: [(u8, edge::Meta, edge::Data); LEN] =
            core::array::from_fn(|_| (0, edge::Meta::default(), edge::Data::default()));
        let mut len = 0;

        self.edges
            .iter()
            .map(|edge| (edge, edge.load_low(Ordering::Relaxed)))
            .zip(self.header.keys())
            .inspect(|((_, meta), _)| {
                if cfg!(feature = "validate") {
                    assert!(
                        meta.frozen,
                        "{} edge must be frozen before replace",
                        core::any::type_name::<Self>(),
                    )
                }
            })
            .filter(|((_, meta), _)| !matches!(meta.kind, node::Kind::None))
            .map(|((edge, meta), key)| (key, meta.unfreeze(), edge.load_high(Ordering::Relaxed)))
            .zip(&mut edges)
            .for_each(|(edge, save)| {
                *save = edge;
                len += 1;
            });

        match &edges[..len] {
            [] => (
                Op::Destroy,
                edge::Meta {
                    key: key::Array::default(),
                    kind: node::Kind::None,
                    frozen: false,
                },
                edge::Data::default(),
            ),

            [(key, meta, data)] if key::Array::can_compress(&snapshot.key, &meta.key) => (
                Op::Compress,
                edge::Meta {
                    key: unsafe { key::Array::compress(&snapshot.key, *key, &meta.key) },
                    kind: snapshot.kind,
                    frozen: false,
                },
                *data,
            ),

            // Grow
            _ if len == <Self as node::Info>::GROW => (
                node::Op::Grow,
                edge::Meta {
                    key: snapshot.key,
                    kind: <<Self as node::Info>::Grow as node::Info>::KIND,
                    frozen: false,
                },
                edge::Data::new_node::<<Self as node::Info>::Grow, _>(edges.into_iter().take(len)),
            ),

            // Replace
            _ => (
                node::Op::Replace,
                edge::Meta {
                    key: snapshot.key,
                    kind: <Self as node::Info>::KIND,
                    frozen: false,
                },
                edge::Data::new_node::<Self, _>(edges.into_iter().take(len)),
            ),
        }
    }
}

pub(super) trait Header {
    fn is_frozen(&self) -> bool;
    fn freeze(&self) -> usize;
    fn get(&self, key: u8) -> usize;
    fn get_or_reserve(&self, key: u8) -> Result<usize, Frozen>;
    fn keys(&self) -> super::KeyIter;
}
