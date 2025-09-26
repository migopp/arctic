use core::fmt::Debug;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;
use ribbit::Unpack as _;

use crate::byte;
use crate::edge;
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
            self.header.keys_unsorted(),
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
                if let Some(compress) = byte::Array::compress(parent.key(), *key, edge.meta().key())
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

#[expect(private_bounds)]
impl<const LEN: usize, H: Header> Linear<LEN, H> {
    pub(super) fn iter_sorted(&self) -> SortedIter {
        SortedIter {
            keys: self.header.keys_sorted(),
            edges: self.edges.as_slice(),
        }
    }

    pub(super) fn iter_unsorted(&self) -> UnsortedIter {
        self.header.keys_unsorted().zip(self.edges.as_slice())
    }
}

pub(super) trait Header {
    fn is_frozen(&self) -> bool;
    fn freeze(&self) -> usize;
    fn get(&self, key: u8) -> Option<u8>;
    fn get_or_reserve(&self, key: u8) -> Option<u8>;

    fn keys_sorted(&self) -> SortedKeyIter;
    fn keys_unsorted(&self) -> UnsortedKeyIter;
}

pub(crate) struct SortedIter<'a> {
    keys: SortedKeyIter,
    edges: &'a [Atomic128<Edge>],
}

impl<'a> Iterator for SortedIter<'a> {
    type Item = (u8, &'a Atomic128<Edge>);
    fn next(&mut self) -> Option<Self::Item> {
        let (key, index) = self.keys.next()?;
        self.edges.get(index as usize).map(|edge| (key, edge))
    }
}

pub(crate) type UnsortedIter<'a> =
    core::iter::Zip<UnsortedKeyIter, core::slice::Iter<'a, Atomic128<Edge>>>;

pub(crate) enum SortedKeyIter {
    K3(core::iter::Take<core::array::IntoIter<(u8, u8), 3>>),
    K15(core::iter::Take<core::array::IntoIter<(u8, u8), 15>>),
}

impl SortedKeyIter {
    pub(crate) fn new_3(keys: u32, len: usize) -> Self {
        let keys = keys.to_ne_bytes();
        let mut indexes: [(u8, u8); 3] = core::array::from_fn(|index| (keys[index], index as u8));
        indexes[..len].sort_unstable();
        Self::K3(indexes.into_iter().take(len))
    }

    pub(crate) fn new_15(keys: u128, len: usize) -> Self {
        let keys = keys.to_ne_bytes();
        let mut indexes: [(u8, u8); 15] = core::array::from_fn(|index| (keys[index], index as u8));
        indexes[..len].sort_unstable();
        Self::K15(indexes.into_iter().take(len))
    }
}

impl Iterator for SortedKeyIter {
    type Item = (u8, u8);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            SortedKeyIter::K3(iter) => iter.next(),
            SortedKeyIter::K15(iter) => iter.next(),
        }
    }
}

pub(crate) enum UnsortedKeyIter {
    K3(core::iter::Take<core::array::IntoIter<u8, 4>>),
    K15(core::iter::Take<core::array::IntoIter<u8, 16>>),
}

impl UnsortedKeyIter {
    pub(crate) fn new_3(keys: u32, len: usize) -> Self {
        Self::K3(keys.to_ne_bytes().into_iter().take(len))
    }

    pub(crate) fn new_15(keys: u128, len: usize) -> Self {
        Self::K15(keys.to_ne_bytes().into_iter().take(len))
    }
}

impl Iterator for UnsortedKeyIter {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::K3(iter) => iter.next(),
            Self::K15(iter) => iter.next(),
        }
    }
}
