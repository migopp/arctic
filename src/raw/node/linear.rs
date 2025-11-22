use core::fmt::Debug;
use core::sync::atomic::Ordering;

use ribbit::Atomic;

use crate::raw::edge;
use crate::raw::node;
use crate::raw::node::Edge;
use crate::raw::Node;

#[repr(C, align(64))]
pub(crate) struct Linear<const LEN: usize, H: ribbit::Pack, M: ribbit::Pack> {
    pub(super) header: Atomic<H>,
    pub(super) edges: [Atomic<Edge<M>>; LEN],
}

impl<const LEN: usize, H, M> Default for Linear<LEN, H, M>
where
    H: ribbit::Pack<Packed: Default>,
    M: ribbit::Pack<Packed: edge::Meta>,
{
    fn default() -> Self {
        Self {
            header: Atomic::new_packed(H::Packed::default()),
            edges: core::array::from_fn(|_| Atomic::new_packed(Edge::DEFAULT)),
        }
    }
}

impl<const LEN: usize, H, M> Node<M> for Linear<LEN, H, M>
where
    H: ribbit::Pack<Packed: Header + Default>,
    M: ribbit::Pack<Packed: edge::Meta>,
{
    const KIND: node::Kind = <H::Packed as Header>::KIND;
    const GROW: usize = <H::Packed as Header>::GROW;

    type Grow = <H::Packed as Header>::Grow<M>;
    type Shrink = <H::Packed as Header>::Shrink<M>;

    type KeyBuffer = <H::Packed as Header>::KeyBuffer;
    type EdgeBuffer = <H::Packed as Header>::EdgeBuffer<M>;

    fn buffer() -> (Self::KeyBuffer, Self::EdgeBuffer) {
        <H::Packed as Header>::buffer::<M>()
    }

    #[inline]
    fn edges(&self) -> &[Atomic<Edge<M>>] {
        &self.edges
    }

    #[inline]
    fn get(&self, key: u8) -> Option<&Atomic<Edge<M>>> {
        let header = self.header.load_packed(Ordering::Relaxed);
        let index = header.get(key)?;
        validate!((index as usize) < self.edges.len());
        Some(unsafe { self.edges.get_unchecked(index as usize) })
    }

    #[inline]
    fn get_or_insert(&self, key: u8) -> Option<&Atomic<Edge<M>>> {
        let mut old = self.header.load_packed(Ordering::Relaxed);

        let index = loop {
            let new = match old.get_or_insert(key) {
                Ok(index) => break index as usize,
                Err(None) => return None,
                Err(Some(new)) => new,
            };

            match self.header.compare_exchange_packed(
                old,
                new,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break old.len(),
                Err(conflict) if conflict.is_frozen() => return None,
                Err(conflict) => old = conflict,
            }
        };

        validate!(index < self.edges.len());
        Some(unsafe { self.edges.get_unchecked(index) })
    }

    #[inline]
    fn insert(&mut self, key: u8) -> Option<&mut Atomic<Edge<M>>> {
        let old = self.header.get_packed();

        let index = match old.get_or_insert(key) {
            Ok(index) => index as usize,
            Err(None) => return None,
            Err(Some(new)) => {
                self.header.set_packed(new);
                old.len()
            }
        };

        validate!(index < self.edges.len());
        Some(unsafe { self.edges.get_unchecked_mut(index) })
    }

    fn freeze(
        &self,
    ) -> (
        impl Iterator<Item = u8>,
        impl Iterator<Item = ribbit::Packed<Edge<M>>>,
    ) {
        let header = Linear::freeze(self);
        (
            header.keys_unsorted().map(|(key, _)| key),
            self.edges
                .iter()
                .map(|edge| edge.load_packed(Ordering::Relaxed)),
        )
    }
}

impl<const LEN: usize, H, M> Linear<LEN, H, M>
where
    H: ribbit::Pack<Packed: Header>,
    M: ribbit::Pack<Packed: edge::Meta>,
{
    fn freeze(&self) -> ribbit::Packed<H> {
        let mut header = self.header.load_packed(Ordering::Relaxed);

        while !header.is_frozen() {
            match self.header.compare_exchange_packed(
                header,
                header.freeze(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(conflict) => header = conflict,
            }
        }

        self.edges.iter().take(header.len()).for_each(Edge::freeze);
        header
    }

    // FIXME
    #[inline]
    pub(crate) fn keys_range<L: crate::raw::node::Lower, G: crate::raw::node::Upper>(
        &self,
        low: L,
        high: G,
    ) -> node::KeyIter {
        self.header
            .load_packed(Ordering::Relaxed)
            .keys_range(low, high)
    }

    #[inline]
    pub(crate) fn keys_sorted(&self) -> node::KeyIter {
        self.header.load_packed(Ordering::Relaxed).keys_sorted()
    }

    #[inline]
    pub(crate) fn keys_unsorted(&self) -> node::KeyIter {
        self.header.load_packed(Ordering::Relaxed).keys_unsorted()
    }
}

impl<const LEN: usize, H, M> Debug for Linear<LEN, H, M>
where
    H: ribbit::Pack<Packed: Debug>,
    M: ribbit::Pack<Packed: edge::Meta + Debug>,
{
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

pub(crate) trait Header: ribbit::Unpack {
    const KIND: node::Kind;
    const GROW: usize;

    type Grow<M>: Node<M>
    where
        M: ribbit::Pack<Packed: edge::Meta>;

    type Shrink<M>: Node<M>
    where
        M: ribbit::Pack<Packed: edge::Meta>;

    type KeyBuffer: AsMut<[u8]>;
    type EdgeBuffer<M>: AsMut<[ribbit::Packed<Edge<M>>]>
    where
        M: ribbit::Pack<Packed: edge::Meta>;

    fn buffer<M: ribbit::Pack<Packed: edge::Meta>>() -> (Self::KeyBuffer, Self::EdgeBuffer<M>);

    fn freeze(self) -> Self;

    fn is_frozen(self) -> bool;

    fn len(self) -> usize;

    fn get(self, key: u8) -> Option<u8>;

    fn get_or_insert(self, key: u8) -> Result<u8, Option<Self>>;

    fn keys_range<L: crate::raw::node::Lower, H: crate::raw::node::Upper>(
        self,
        low: L,
        high: H,
    ) -> node::KeyIter;

    fn keys_sorted(self) -> node::KeyIter;

    fn keys_unsorted(self) -> node::KeyIter;
}

#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct KeyIter<const N: usize> {
    head: u8,
    inner: [(u8, u8); N],
    tail: u8,
}

const _: [(); 0] = [(); core::mem::offset_of!(KeyIter::<3>, head)];
const _: [(); 8] = [(); core::mem::size_of::<KeyIter<3>>()];
const _: [(); 32] = [(); core::mem::size_of::<KeyIter<15>>()];

impl<const N: usize> KeyIter<N> {
    #[inline]
    pub(super) const fn new(inner: [(u8, u8); N], len: u8) -> Self {
        Self {
            inner,
            head: 0,
            tail: len,
        }
    }
}

impl<const N: usize> Iterator for KeyIter<N> {
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

impl<const N: usize> DoubleEndedIterator for KeyIter<N> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.head == self.tail {
            return None;
        }

        self.tail -= 1;
        self.inner.get(self.tail as usize).copied()
    }
}

impl<const N: usize> ExactSizeIterator for KeyIter<N> {
    #[inline]
    fn len(&self) -> usize {
        let (lower, upper) = self.size_hint();
        validate_eq!(upper, Some(lower));
        lower
    }
}
