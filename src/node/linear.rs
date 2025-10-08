use core::fmt::Debug;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::edge;
use crate::iter::Or;
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

    fn replace(&self, parent: ribbit::Packed<edge::Meta>) -> (Op, ribbit::Packed<Edge>) {
        let len = self.header.freeze();
        self.edges.iter().take(len).for_each(Edge::freeze);

        let mut edges: [(u8, ribbit::Packed<Edge>); LEN] =
            core::array::from_fn(|_| (0, Edge::DEFAULT));
        let mut len = 0;

        core::iter::zip(
            self.header.keys_unsorted(),
            self.edges
                .iter()
                .map(|edge| edge.load_packed(Ordering::Relaxed)),
        )
        .filter(|(_, edge)| edge.meta().leaf() || edge.data() != 0)
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
                if let Some(compress) = parent.key().compress(*key, edge.meta().key()) {
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
    #[inline]
    pub(crate) fn iter(&self) -> Iter {
        Iter {
            keys: self.header.keys(),
            edges: self.edges.as_slice(),
        }
    }

    #[inline]
    pub(crate) fn iter_range(&self, min: u8, max: u8) -> RangeIter {
        Iter {
            keys: self.header.keys_range(min, max),
            edges: self.edges.as_slice(),
        }
    }

    #[inline]
    pub(crate) fn iter_unsorted(&self) -> UnsortedIter {
        UnsortedIter {
            keys: self.header.keys_unsorted(),
            edges: self.edges.iter(),
        }
    }
}

pub(super) trait Header {
    fn freeze(&self) -> usize;
    fn get(&self, key: u8) -> Option<u8>;
    fn get_or_reserve(&self, key: u8) -> Option<u8>;

    fn keys(&self) -> KeyIter;
    fn keys_range(&self, min: u8, max: u8) -> RangeKeyIter;
    fn keys_unsorted(&self) -> UnsortedKeyIter;
}

pub(crate) struct Iter<'a> {
    keys: KeyIter,
    edges: &'a [Atomic128<Edge>],
}

impl<'a> Iterator for Iter<'a> {
    type Item = (u8, &'a Atomic128<Edge>);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (key, index) = self.keys.next()?;
        let edge = unsafe { self.edges.get_unchecked(index as usize) };
        Some((key, edge))
    }
}

impl<'a> DoubleEndedIterator for Iter<'a> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        let (key, index) = self.keys.next_back()?;
        let edge = unsafe { self.edges.get_unchecked(index as usize) };
        Some((key, edge))
    }
}

pub(crate) struct UnsortedIter<'a> {
    keys: UnsortedKeyIter,
    edges: core::slice::Iter<'a, Atomic128<Edge>>,
}

impl<'a> Iterator for UnsortedIter<'a> {
    type Item = (u8, &'a Atomic128<Edge>);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let key = self.keys.next()?;
        let edge = unsafe { self.edges.next().unwrap_unchecked() };
        Some((key, edge))
    }
}

pub(crate) type RangeIter<'a> = Iter<'a>;

pub(crate) type KeyIter = Or<
    core::iter::Take<core::array::IntoIter<(u8, u8), 3>>,
    core::iter::Take<core::array::IntoIter<(u8, u8), 15>>,
>;

pub(crate) type UnsortedKeyIter = Or<
    core::iter::Take<core::array::IntoIter<u8, 4>>,
    core::iter::Take<core::array::IntoIter<u8, 16>>,
>;

pub(crate) type RangeKeyIter = KeyIter;
