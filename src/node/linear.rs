use core::fmt::Debug;
use core::num::NonZeroUsize;
use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::edge;
use crate::iter::Or;
use crate::node;
use crate::node::Edge;
use crate::node::Op;
use crate::Node;

#[repr(C, align(64))]
pub(crate) struct Linear<const LEN: usize, H, V> {
    pub(super) header: H,
    pub(super) edges: [Atomic128<Edge<V>>; LEN],
}

impl<const LEN: usize, H: Default, V> Default for Linear<LEN, H, V> {
    fn default() -> Self {
        Self {
            header: H::default(),
            edges: core::array::from_fn(|_| Atomic128::default()),
        }
    }
}

impl<const LEN: usize, H, V> Node<V> for Linear<LEN, H, V>
where
    H: Header,
    Self: node::Info<V>,
{
    #[inline]
    fn edges(&self) -> &[Atomic128<Edge<V>>] {
        &self.edges
    }

    #[inline]
    fn get(&self, key: u8) -> Option<&Atomic128<Edge<V>>> {
        let index = self.header.get(key)?;
        Some(unsafe { self.edges.get_unchecked(index as usize) })
    }

    #[inline]
    fn get_or_reserve(&self, key: u8) -> Option<&Atomic128<Edge<V>>> {
        let index = self.header.get_or_reserve(key)?;
        Some(unsafe { self.edges.get_unchecked(index as usize) })
    }

    #[inline]
    fn reserve(&mut self, key: u8) -> Option<&mut Atomic128<Edge<V>>> {
        let index = self.header.get_or_reserve(key)?;
        Some(unsafe { self.edges.get_unchecked_mut(index as usize) })
    }

    fn replace(&self, parent: ribbit::Packed<edge::Meta>) -> (Op, ribbit::Packed<Edge<V>>) {
        let len = self.header.freeze();
        self.edges.iter().take(len).for_each(Edge::freeze);

        let mut edges: [(u8, ribbit::Packed<Edge<V>>); LEN] =
            core::array::from_fn(|_| (0, Edge::DEFAULT));
        let mut len = 0;

        core::iter::zip(
            self.header.keys_unsorted(),
            self.edges
                .iter()
                .map(|edge| edge.load_packed(Ordering::Relaxed)),
        )
        .filter(|(_, edge)| !edge.is_null())
        .map(|(key, edge)| {
            validate!(
                edge.meta().is_frozen(),
                "{} edge must be frozen before replace",
                core::any::type_name::<Self>(),
            );
            (key, edge.unfreeze())
        })
        .zip(&mut edges)
        .for_each(|(edge, save)| {
            *save = edge;
            len += 1;
        });

        match &edges[..len] {
            _ if len == <Self as node::Info<V>>::GROW => {
                return (
                    node::Op::Grow,
                    Edge::new_node::<<Self as node::Info<V>>::Grow, _>(
                        parent.key(),
                        edges.into_iter().take(len),
                    ),
                )
            }
            [] => return (Op::Destroy, Edge::DEFAULT),
            [(key, edge)] => {
                // FIXME: how to handle scan?
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
impl<const LEN: usize, H: Header, V> Linear<LEN, H, V> {
    #[inline]
    pub(crate) fn keys_sorted(&self) -> SortedKeyIter {
        self.header.keys_sorted()
    }

    #[inline]
    pub(crate) fn keys_range(&self, min: u8, max: u8) -> SortedKeyIter {
        self.header.keys_range(min, max)
    }

    #[inline]
    pub(crate) fn keys_unsorted(&self) -> UnsortedKeyIter {
        self.header.keys_unsorted()
    }
}

impl<const LEN: usize, H: Debug, V> Debug for Linear<LEN, H, V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let name = const {
            if LEN == 3 {
                "Node3"
            } else if LEN == 15 {
                "Node15"
            } else {
                unreachable!()
            }
        };

        f.debug_struct(name)
            .field("header", &self.header)
            .field("edges", &edge::DebugSlice(&self.edges))
            .finish()
    }
}

pub(super) trait Header {
    fn freeze(&self) -> usize;
    fn get(&self, key: u8) -> Option<u8>;
    fn get_or_reserve(&self, key: u8) -> Option<u8>;

    fn keys_sorted(&self) -> SortedKeyIter;
    fn keys_range(&self, min: u8, max: u8) -> SortedKeyIter;
    fn keys_unsorted(&self) -> UnsortedKeyIter;
}

pub(crate) type UnsortedKeyIter = Or<
    core::iter::Take<core::array::IntoIter<u8, 4>>,
    core::iter::Take<core::array::IntoIter<u8, 16>>,
>;

pub(crate) union SortedKeyIter {
    node_3: RawKeyIter<3>,
    node_15: NonNull<RawKeyIter<15>>,
    raw: usize,
}

const _: [(); size_of::<usize>()] = [(); size_of::<RawKeyIter<3>>()];
const _: [(); size_of::<usize>()] = [(); size_of::<NonNull<RawKeyIter<15>>>()];

impl SortedKeyIter {
    const MASK_TAG: usize = 0b100
        << if cfg!(target_endian = "little") {
            56
        } else {
            0
        };
    const MASK_PTR: usize = !Self::MASK_TAG;

    #[inline]
    pub(super) fn new_3(iter: RawKeyIter<3>) -> Self {
        Self { node_3: iter }
    }

    #[inline]
    pub(super) fn new_15(iter: RawKeyIter<15>) -> Self {
        Self {
            node_15: NonNull::from(Box::leak(Box::new(iter))).map_addr(|addr| unsafe {
                validate_eq!(addr.get() & Self::MASK_TAG, 0);
                NonZeroUsize::new_unchecked(addr.get() | Self::MASK_TAG)
            }),
        }
    }

    #[inline]
    fn is_node_3(&self) -> bool {
        unsafe { self.raw & Self::MASK_TAG == 0 }
    }

    #[inline]
    fn with<N3, N15, T>(&self, node_3: N3, node_15: N15) -> T
    where
        N3: FnOnce(&RawKeyIter<3>) -> T,
        N15: FnOnce(&RawKeyIter<15>) -> T,
    {
        if self.is_node_3() {
            node_3(unsafe { &self.node_3 })
        } else {
            crate::cold();
            node_15(unsafe {
                self.node_15
                    .map_addr(|addr| NonZeroUsize::new_unchecked(addr.get() & Self::MASK_PTR))
                    .as_ref()
            })
        }
    }

    #[inline]
    fn with_mut<N3, N15, T>(&mut self, node_3: N3, node_15: N15) -> T
    where
        N3: FnOnce(&mut RawKeyIter<3>) -> T,
        N15: FnOnce(&mut RawKeyIter<15>) -> T,
    {
        if self.is_node_3() {
            node_3(unsafe { &mut self.node_3 })
        } else {
            crate::cold();
            node_15(unsafe {
                self.node_15
                    .map_addr(|addr| NonZeroUsize::new_unchecked(addr.get() & Self::MASK_PTR))
                    .as_mut()
            })
        }
    }
}

impl Clone for SortedKeyIter {
    fn clone(&self) -> Self {
        self.with(
            |node_3| Self { node_3: *node_3 },
            |node_15| Self::new_15(*node_15),
        )
    }
}

impl Drop for SortedKeyIter {
    #[inline]
    fn drop(&mut self) {
        if self.is_node_3() {
            return;
        }

        crate::cold();
        unsafe {
            drop(Box::from_raw(
                self.node_15
                    .map_addr(|addr| NonZeroUsize::new_unchecked(addr.get() & Self::MASK_PTR))
                    .as_ptr(),
            ))
        }
    }
}

const _: [(); 8] = [(); size_of::<SortedKeyIter>()];

impl Iterator for SortedKeyIter {
    type Item = (u8, u8);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.with_mut(|node_3| node_3.next(), |node_15| node_15.next())
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.with(|node_3| node_3.size_hint(), |node_15| node_15.size_hint())
    }
}

impl DoubleEndedIterator for SortedKeyIter {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.with_mut(|node_3| node_3.next_back(), |node_15| node_15.next_back())
    }
}

impl ExactSizeIterator for SortedKeyIter {
    #[inline]
    fn len(&self) -> usize {
        let (lower, upper) = self.size_hint();
        validate_eq!(upper, Some(lower));
        lower
    }
}

#[repr(C)]
#[derive(Copy, Clone)]
pub(super) struct RawKeyIter<const N: usize> {
    head: u8,
    inner: [(u8, u8); N],
    tail: u8,
}

const _: [(); 0] = [(); core::mem::offset_of!(RawKeyIter::<3>, head)];

impl<const N: usize> RawKeyIter<N> {
    #[inline]
    pub(super) fn new(inner: [(u8, u8); N], len: u8) -> Self {
        Self {
            inner,
            head: 0,
            tail: len,
        }
    }
}

impl<const N: usize> Iterator for RawKeyIter<N> {
    type Item = (u8, u8);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.head == self.tail {
            return None;
        }

        let next = self.inner.get(self.head as usize).copied()?;
        self.head += 1;
        Some(next)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = (self.tail - self.head) as usize;
        (len, Some(len))
    }
}

impl<const N: usize> DoubleEndedIterator for RawKeyIter<N> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.head == self.tail {
            return None;
        }

        self.tail -= 1;
        self.inner.get(self.tail as usize).copied()
    }
}

impl<const N: usize> ExactSizeIterator for RawKeyIter<N> {
    #[inline]
    fn len(&self) -> usize {
        let (lower, upper) = self.size_hint();
        validate_eq!(upper, Some(lower));
        lower
    }
}
