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

unsafe impl<const LEN: usize, H, M> Node<M> for Linear<LEN, H, M>
where
    H: ribbit::Pack<Packed: Header + Default>,
    M: ribbit::Pack<Packed: edge::Meta>,
{
    const KIND: node::Kind = <H::Packed as Header>::KIND;
    const LEN: usize = <H::Packed as Header>::LEN;

    type Grow = <H::Packed as Header>::Grow<M>;
    type Shrink = <H::Packed as Header>::Shrink<M>;

    #[inline]
    fn keys<L: crate::raw::node::Lower, U: crate::raw::node::Upper>(
        &self,
        lower: L,
        upper: U,
    ) -> node::KeyIter {
        self.header
            .load_packed(Ordering::Relaxed)
            .keys(lower, upper)
    }

    #[inline]
    fn edges(&self) -> &[Atomic<Edge<M>>] {
        &self.edges
    }

    #[inline]
    fn edges_mut(&mut self) -> &mut [Atomic<Edge<M>>] {
        &mut self.edges
    }

    #[inline]
    fn get_key(&self, key: u8) -> Option<u8> {
        self.header.load_packed(Ordering::Relaxed).get(key)
    }

    #[inline]
    fn get_or_insert_key(&self, key: u8) -> Option<u8> {
        let mut old = self.header.load_packed(Ordering::Relaxed);

        loop {
            let new = match old.get_or_insert(key) {
                Ok(index) => return Some(index),
                Err(None) => return None,
                Err(Some(new)) => new,
            };

            match self.header.compare_exchange_packed(
                old,
                new,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break Some(old.len()),
                Err(conflict) => old = conflict,
            }
        }
    }

    #[inline]
    fn insert_key(&mut self, key: u8) -> Option<u8> {
        let old = self.header.get_packed();

        match old.get_or_insert(key) {
            Ok(index) => Some(index),
            Err(None) => None,
            Err(Some(new)) => {
                self.header.set_packed(new);
                Some(old.len())
            }
        }
    }

    fn freeze(&self) {
        Linear::freeze(self);
    }
}

impl<const LEN: usize, H, M> Linear<LEN, H, M>
where
    H: ribbit::Pack<Packed: Header>,
    M: ribbit::Pack<Packed: edge::Meta>,
{
    fn freeze(&self) {
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

        self.edges
            .iter()
            .take(header.len() as usize)
            .for_each(Edge::freeze);
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
    const LEN: usize;

    type Grow<M>: Node<M>
    where
        M: ribbit::Pack<Packed: edge::Meta>;

    type Shrink<M>: Node<M>
    where
        M: ribbit::Pack<Packed: edge::Meta>;

    fn freeze(self) -> Self;

    fn is_frozen(self) -> bool;

    fn len(self) -> u8;

    fn get(self, key: u8) -> Option<u8>;

    fn get_or_insert(self, key: u8) -> Result<u8, Option<Self>>;

    fn keys<L: crate::raw::node::Lower, U: crate::raw::node::Upper>(
        self,
        lower: L,
        upper: U,
    ) -> node::KeyIter;
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub(super) struct KeyIter3 {
    head: u8,
    entries: [node::iter::KeyIndex; 3],
    tail: u8,
}

impl KeyIter3 {
    #[inline]
    pub(super) fn try_into_single(self) -> Option<node::iter::KeyIndex> {
        (self.tail == 1).then_some(self.entries[0])
    }
}

const _: [(); 8] = [(); core::mem::size_of::<KeyIter3>()];

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub(super) struct KeyIter<const N: usize> {
    head: u8,
    tail: u8,
    entries: [node::iter::KeyIndex; N],
}

const _: [(); 32] = [(); core::mem::size_of::<KeyIter<15>>()];
const _: [(); 96] = [(); core::mem::size_of::<KeyIter<47>>()];

macro_rules! impl_key_iter {
    ($ty:ty, $len:expr, $new:ident) => {
        impl $ty {
            #[inline]
            pub(super) const fn $new(entries: [node::iter::KeyIndex; $len], len: u8) -> Self {
                validate!(len as usize <= entries.len());
                Self {
                    head: 0,
                    tail: len,
                    entries,
                }
            }
        }

        impl Iterator for $ty {
            type Item = node::iter::KeyIndex;
            #[inline]
            fn next(&mut self) -> Option<Self::Item> {
                if self.head == self.tail {
                    return None;
                }

                let next = self.entries.get(self.head as usize).copied()?;
                self.head += 1;
                Some(next)
            }

            #[inline]
            fn size_hint(&self) -> (usize, Option<usize>) {
                let len = (self.tail - self.head) as usize;
                (len, Some(len))
            }
        }

        impl DoubleEndedIterator for $ty {
            #[inline]
            fn next_back(&mut self) -> Option<Self::Item> {
                if self.head == self.tail {
                    return None;
                }

                self.tail -= 1;
                self.entries.get(self.tail as usize).copied()
            }
        }

        impl ExactSizeIterator for $ty {
            #[inline]
            fn len(&self) -> usize {
                let (lower, upper) = self.size_hint();
                validate_eq!(upper, Some(lower));
                lower
            }
        }
    };
}

impl_key_iter!(KeyIter3, 3, new_3);
impl_key_iter!(KeyIter<15>, 15, new_15);
impl_key_iter!(KeyIter<47>, 47, new_47);
