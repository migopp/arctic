use core::fmt::Debug;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;
use ribbit::Unpack as _;

use crate::edge;
use crate::key;
use crate::node;
use crate::node::Edge;
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
    #[inline]
    fn get(&self, key: u8) -> Option<&Atomic128<Edge>> {
        let index = self.header.get(key)?;
        Some(unsafe { self.edges.get_unchecked(index as usize) })
    }

    #[inline]
    fn get_or_reserve(&self, key: u8) -> Option<&Atomic128<Edge>> {
        let index = self.header.get_or_reserve(key)?;
        Some(unsafe { self.edges.get_unchecked(index as usize) })
    }

    #[inline]
    fn reserve(&mut self, key: u8) -> Option<&mut Atomic128<Edge>> {
        let index = self.header.get_or_reserve(key)?;
        Some(unsafe { self.edges.get_unchecked_mut(index as usize) })
    }

    fn freeze(&self) {
        let len = self.header.freeze();
        self.edges.iter().take(len).for_each(Edge::freeze);
    }

    fn replace(&self, parent: ribbit::Packed<edge::Meta>) -> (Op, ribbit::Packed<Edge>) {
        validate!(
            self.header.is_frozen(),
            "{} header must be frozen before replace",
            core::any::type_name::<Self>(),
        );

        let mut edges: [(u8, ribbit::Packed<Edge>); LEN] =
            core::array::from_fn(|_| (0, Edge::DEFAULT));
        let mut len = 0;

        core::iter::zip(
            self.header.keys(),
            self.edges
                .iter()
                .map(|edge| edge.load_packed(Ordering::Relaxed)),
        )
        .filter(|(_, edge)| !matches!(edge.meta().kind().unpack(), node::Kind::None))
        .map(|(key, edge)| {
            validate!(
                edge.meta().frozen(),
                "{} edge must be frozen before replace",
                core::any::type_name::<Self>(),
            );
            (key, edge.with_meta(edge.meta().with_frozen(false)))
        })
        .zip(&mut edges)
        .for_each(|(edge, save)| {
            *save = edge;
            len += 1;
        });

        match &edges[..len] {
            _ if len == <Self as node::Info>::GROW => {
                return (
                    node::Op::Grow,
                    Edge::new_node::<<Self as node::Info>::Grow, _>(
                        parent.key(),
                        edges.into_iter().take(len),
                    ),
                )
            }
            [] => return (Op::Destroy, Edge::DEFAULT),
            [(key, edge)] => {
                if let Some(compress) = key::Array::compress(parent.key(), *key, edge.meta().key())
                {
                    return (Op::Compress, edge.with_meta(edge.meta().with_key(compress)));
                }
            }

            _ => (),
        }

        // Catch-all:
        (
            node::Op::Replace,
            Edge::new_node::<Self, _>(parent.key(), edges.into_iter().take(len)),
        )
    }
}

pub(super) trait Header {
    fn is_frozen(&self) -> bool;
    fn freeze(&self) -> usize;
    fn get(&self, key: u8) -> Option<u8>;
    fn get_or_reserve(&self, key: u8) -> Option<u8>;
    fn keys(&self) -> super::KeyIter;
}
