use core::fmt::Debug;
use core::num::NonZeroUsize;
use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::raw::edge;
use crate::raw::node;
use crate::raw::node::Edge;
use crate::raw::node::Op;
use crate::raw::Node;

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

impl<const LEN: usize, H, C> Node<C> for Linear<LEN, H, C>
where
    H: Header<C>,
{
    const KIND: node::Kind = H::KIND;
    const GROW: usize = H::GROW;

    type Grow = H::Grow;
    type Shrink = H::Shrink;

    #[inline]
    fn edges(&self) -> &[Atomic128<Edge<C>>] {
        &self.edges
    }

    #[inline]
    fn get(&self, key: u8) -> Option<&Atomic128<Edge<C>>> {
        let index = self.header.get(key)?;
        Some(unsafe { self.edges.get_unchecked(index as usize) })
    }

    #[inline]
    fn get_or_reserve(&self, key: u8) -> Option<&Atomic128<Edge<C>>> {
        let index = self.header.get_or_reserve(key)?;
        Some(unsafe { self.edges.get_unchecked(index as usize) })
    }

    #[inline]
    fn reserve(&mut self, key: u8) -> Option<&mut Atomic128<Edge<C>>> {
        let index = self.header.get_or_reserve(key)?;
        Some(unsafe { self.edges.get_unchecked_mut(index as usize) })
    }

    fn replace(&self, parent: ribbit::Packed<edge::Meta>) -> (Op, ribbit::Packed<Edge<C>>) {
        let len = self.header.freeze();
        self.edges.iter().take(len).for_each(Edge::freeze);

        let mut edges: [(u8, ribbit::Packed<Edge<C>>); LEN] =
            core::array::from_fn(|_| (0, Edge::DEFAULT));
        let mut len = 0;

        core::iter::zip(
            self.header.keys_unsorted().map(|(key, _)| key),
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
            _ if len == Self::GROW => {
                return (
                    node::Op::Grow,
                    Edge::new_node::<Self::Grow, _>(parent.key(), edges.into_iter().take(len)),
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

impl<const LEN: usize, H: Header<V>, V> Linear<LEN, H, V> {
    // FIXME
    #[inline]
    pub(crate) fn keys_range<L: crate::raw::node::Low, G: crate::raw::node::High>(
        &self,
        low: L,
        high: G,
    ) -> KeyIter {
        self.header.keys_range(low, high)
    }

    #[inline]
    pub(crate) fn keys_sorted(&self) -> KeyIter {
        self.header.keys_sorted()
    }

    #[inline]
    pub(crate) fn keys_unsorted(&self) -> KeyIter {
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
            .field("edges", &self.edges)
            .finish()
    }
}

pub(crate) trait Header<C>: Default {
    const KIND: node::Kind = node::Kind::Node3;
    const GROW: usize = 3;

    type Grow: Node<C>;
    type Shrink: Node<C>;

    fn freeze(&self) -> usize;
    fn get(&self, key: u8) -> Option<u8>;
    fn get_or_reserve(&self, key: u8) -> Option<u8>;

    fn keys_range<L: crate::raw::node::Low, H: crate::raw::node::High>(
        &self,
        low: L,
        high: H,
    ) -> KeyIter;

    fn keys_sorted(&self) -> KeyIter;

    fn keys_unsorted(&self) -> KeyIter;
}

pub(crate) union KeyIter {
    node_3: RawIter<3>,
    node_15: NonNull<RawIter<15>>,
    raw: usize,
}

const _: [(); size_of::<usize>()] = [(); size_of::<RawIter<3>>()];
const _: [(); size_of::<usize>()] = [(); size_of::<NonNull<RawIter<15>>>()];

impl KeyIter {
    const MASK_TAG: usize = 0b100
        << if cfg!(target_endian = "little") {
            56
        } else {
            0
        };
    const MASK_PTR: usize = !Self::MASK_TAG;

    pub(super) const ROOT: Self = Self {
        node_3: RawIter {
            head: 0,
            inner: [(0, 0); 3],
            tail: 1,
        },
    };

    #[inline]
    pub(super) fn new_3(iter: RawIter<3>) -> Self {
        Self { node_3: iter }
    }

    #[inline]
    pub(super) fn new_15(iter: RawIter<15>) -> Self {
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
        N3: FnOnce(&RawIter<3>) -> T,
        N15: FnOnce(&RawIter<15>) -> T,
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
        N3: FnOnce(&mut RawIter<3>) -> T,
        N15: FnOnce(&mut RawIter<15>) -> T,
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

impl Drop for KeyIter {
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

const _: [(); 8] = [(); size_of::<KeyIter>()];

impl Iterator for KeyIter {
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

impl DoubleEndedIterator for KeyIter {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.with_mut(|node_3| node_3.next_back(), |node_15| node_15.next_back())
    }
}

impl ExactSizeIterator for KeyIter {
    #[inline]
    fn len(&self) -> usize {
        let (lower, upper) = self.size_hint();
        validate_eq!(upper, Some(lower));
        lower
    }
}

#[repr(C)]
#[derive(Copy, Clone)]
pub(super) struct RawIter<const N: usize> {
    head: u8,
    inner: [(u8, u8); N],
    tail: u8,
}

const _: [(); 0] = [(); core::mem::offset_of!(RawIter::<3>, head)];

impl<const N: usize> RawIter<N> {
    #[inline]
    pub(super) fn new(inner: [(u8, u8); N], len: u8) -> Self {
        Self {
            inner,
            head: 0,
            tail: len,
        }
    }
}

impl<const N: usize> Iterator for RawIter<N> {
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

impl<const N: usize> DoubleEndedIterator for RawIter<N> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.head == self.tail {
            return None;
        }

        self.tail -= 1;
        self.inner.get(self.tail as usize).copied()
    }
}

impl<const N: usize> ExactSizeIterator for RawIter<N> {
    #[inline]
    fn len(&self) -> usize {
        let (lower, upper) = self.size_hint();
        validate_eq!(upper, Some(lower));
        lower
    }
}
