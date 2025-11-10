use core::marker::PhantomData;
use core::mem::ManuallyDrop;
use core::ptr::NonNull;

use ribbit::Atomic;

use crate::raw::iter::Unbound;
use crate::raw::node::linear;
use crate::raw::node::node256;
use crate::raw::Edge;

pub(crate) struct NodeIter<'g, L, U, M: ribbit::Pack> {
    lower: L,
    upper: U,

    keys: KeyIter,
    edges: NonNull<Atomic<Edge<M>>>,

    #[cfg(feature = "validate")]
    len: u16,

    _slice: PhantomData<&'g [Atomic<Edge<M>>]>,
}

impl<'g, L, U, M: ribbit::Pack> NodeIter<'g, L, U, M> {
    /// # SAFETY
    ///
    /// Caller must guarantee all indices produced by `keys` are < `edges.len()`.
    pub(crate) unsafe fn new(
        lower: L,
        upper: U,
        keys: KeyIter,
        edges: &'g [Atomic<Edge<M>>],
    ) -> Self {
        Self {
            lower,
            upper,

            keys,
            edges: NonNull::from(edges).cast(),

            #[cfg(feature = "validate")]
            len: edges.len() as u16,

            _slice: PhantomData,
        }
    }

    pub(crate) fn lower(&self) -> &L {
        &self.lower
    }

    pub(crate) fn upper(&self) -> &U {
        &self.upper
    }
}

impl<'g, L, U, M: ribbit::Pack> Iterator for NodeIter<'g, L, U, M> {
    type Item = (u8, &'g Atomic<Edge<M>>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (key, index) = self.keys.next()?;

        #[cfg(feature = "validate")]
        validate!(
            (index as u16) < self.len,
            "index is {} but len is {}",
            index,
            self.len,
        );

        let edge = unsafe { self.edges.add(index as usize).as_ref() };
        Some((key, edge))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.keys.size_hint()
    }
}

impl<'g, L, U, M: ribbit::Pack> DoubleEndedIterator for NodeIter<'g, L, U, M> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        let (key, index) = self.keys.next_back()?;

        #[cfg(feature = "validate")]
        validate!(
            (index as u16) < self.len,
            "index is {} but len is {}",
            index,
            self.len,
        );

        let edge = unsafe { self.edges.add(index as usize).as_ref() };
        Some((key, edge))
    }
}

impl<'g, L, U, M: ribbit::Pack> ExactSizeIterator for NodeIter<'g, L, U, M> {
    #[inline]
    fn len(&self) -> usize {
        let (lower, upper) = self.size_hint();
        validate_eq!(upper, Some(lower));
        lower
    }
}

pub(crate) union KeyIter {
    linear: ManuallyDrop<linear::KeyIter>,
    node_256: node256::KeyIter,
    raw: usize,
}

impl KeyIter {
    const MASK_TAG: usize = 1usize.rotate_right(1);

    pub(crate) const ROOT: Self = Self {
        linear: ManuallyDrop::new(linear::KeyIter::ROOT),
    };

    #[inline]
    pub(super) fn from_linear(iter: linear::KeyIter) -> Self {
        let iter = Self {
            linear: ManuallyDrop::new(iter),
        };
        validate_eq!(unsafe { iter.raw } & Self::MASK_TAG, 0);
        iter
    }

    #[inline]
    pub(super) fn from_node_256(iter: node256::KeyIter) -> Self {
        let iter = Self { node_256: iter };
        validate_eq!(unsafe { iter.raw } & Self::MASK_TAG, Self::MASK_TAG);
        iter
    }

    fn is_node_256(&self) -> bool {
        (unsafe { self.raw } & Self::MASK_TAG) > 0
    }
}

impl Iterator for KeyIter {
    type Item = (u8, u8);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.is_node_256() {
            unsafe { &mut self.node_256 }.next().map(|key| (key, key))
        } else {
            unsafe { &mut self.linear }.next()
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.is_node_256() {
            unsafe { &self.node_256 }.size_hint()
        } else {
            unsafe { &self.linear }.size_hint()
        }
    }
}

impl DoubleEndedIterator for KeyIter {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.is_node_256() {
            unsafe { &mut self.node_256 }
                .next_back()
                .map(|key| (key, key))
        } else {
            unsafe { &mut self.linear }.next_back()
        }
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

pub(crate) trait Lower: Copy + Default {
    const UNBOUND: bool;
    fn get(self) -> u8;
    fn is(self, byte: u8) -> bool;
}

pub(crate) trait Upper: Copy + Default {
    const UNBOUND: bool;
    fn get(self) -> u8;
    fn is(self, byte: u8) -> bool;
}

impl Lower for Unbound {
    const UNBOUND: bool = true;
    #[inline]
    fn get(self) -> u8 {
        0
    }
    #[inline]
    fn is(self, _byte: u8) -> bool {
        false
    }
}

impl Upper for Unbound {
    const UNBOUND: bool = true;
    #[inline]
    fn get(self) -> u8 {
        255
    }
    #[inline]
    fn is(self, _byte: u8) -> bool {
        false
    }
}

impl Lower for Option<u8> {
    const UNBOUND: bool = false;
    #[inline]
    fn get(self) -> u8 {
        self.unwrap_or(0)
    }
    #[inline]
    fn is(self, byte: u8) -> bool {
        self == Some(byte)
    }
}

impl Upper for Option<u8> {
    const UNBOUND: bool = false;
    #[inline]
    fn get(self) -> u8 {
        self.unwrap_or(255)
    }
    #[inline]
    fn is(self, byte: u8) -> bool {
        self == Some(byte)
    }
}
