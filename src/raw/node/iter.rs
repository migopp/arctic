use core::marker::PhantomData;
use core::num::NonZeroUsize;
use core::ptr::NonNull;

use ribbit::u2;
use ribbit::Atomic;

use crate::raw::iter::Unbound;
use crate::raw::node;
use crate::raw::node::linear;
use crate::raw::node::node_256;
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
    #[inline]
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

    #[inline]
    pub(crate) fn lower(&self) -> &L {
        &self.lower
    }

    #[inline]
    pub(crate) fn upper(&self) -> &U {
        &self.upper
    }
}

impl<'g, L: Lower, U: Upper, M: ribbit::Pack> NodeIter<'g, L, U, M> {
    #[inline]
    pub(crate) fn try_into_single(self) -> Result<(bool, bool, u8, &'g Atomic<Edge<M>>), Self> {
        self.keys
            .try_into_single()
            .map(|KeyIndex { key, index }| {
                let lower = self.lower.check(key);
                let upper = self.upper.check(key);

                #[cfg(feature = "validate")]
                validate!(
                    (index as u16) < self.len,
                    "index is {} but len is {}",
                    index,
                    self.len,
                );

                let edge = unsafe { self.edges.add(index as usize).as_ref() };
                (lower, upper, key, edge)
            })
            .map_err(|keys| Self {
                lower: self.lower,
                upper: self.upper,
                keys,
                edges: self.edges,
                #[cfg(feature = "validate")]
                len: self.len,
                _slice: PhantomData,
            })
    }
}

impl<'g, L, U, M: ribbit::Pack> Iterator for NodeIter<'g, L, U, M> {
    type Item = (u8, &'g Atomic<Edge<M>>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let KeyIndex { key, index } = self.keys.next()?;

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
        let KeyIndex { key, index } = self.keys.next_back()?;

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

#[repr(C)]
pub(crate) union KeyIter {
    node_3: linear::KeyIter3,
    node_15: NonNull<linear::KeyIter<15>>,
    node_47: NonNull<linear::KeyIter<47>>,
    node_256: node_256::KeyIter,
    raw: u64,
}

#[repr(C)]
#[derive(Copy, Clone, PartialEq, Eq)]
pub(crate) struct KeyIndex {
    pub(super) index: u8,
    pub(super) key: u8,
}

impl KeyIndex {
    pub(crate) const DEFAULT: Self = Self { key: 0, index: 0 };
}

impl core::fmt::Debug for KeyIndex {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:#.02X}:{:#.02X}", self.key, self.index)
    }
}

impl KeyIter {
    pub(crate) const ROOT: Self = Self {
        node_3: linear::KeyIter3::new_3([KeyIndex::DEFAULT; 3], 1),
    };

    const TAG_15: usize = (node::Kind::Node15 as usize) << 62;
    const TAG_47: usize = (node::Kind::Node47 as usize) << 62;

    #[inline]
    fn kind(&self) -> ribbit::Packed<node::Kind> {
        unsafe {
            // SAFETY: shifting u64 by 62 bits, so only 2 bits can remain
            ribbit::Packed::<node::Kind>::new_unchecked(u2::new_unchecked((self.raw >> 62) as u8))
        }
    }

    #[inline]
    pub(super) fn try_into_single(self) -> Result<KeyIndex, Self> {
        let kind = self.kind();
        if kind == node::Kind::NODE_3 {
            if let Some(index) = unsafe { self.node_3 }.try_into_single() {
                return Ok(index);
            }
        }
        Err(self)
    }

    #[inline]
    pub(super) fn new_3(node_3: linear::KeyIter3) -> Self {
        let iter = Self { node_3 };
        validate_eq!(iter.kind(), node::Kind::NODE_3);
        iter
    }

    #[inline]
    pub(super) fn new_15(node_15: Box<linear::KeyIter<15>>) -> Self {
        let iter = Self {
            node_15: NonNull::from(Box::leak(node_15))
                .map_addr(|addr| unsafe { NonZeroUsize::new_unchecked(addr.get() | Self::TAG_15) }),
        };
        validate_eq!(iter.kind(), node::Kind::NODE_15);
        iter
    }

    #[inline]
    pub(super) fn new_47(node_47: Box<linear::KeyIter<47>>) -> Self {
        let iter = Self {
            node_47: NonNull::from(Box::leak(node_47))
                .map_addr(|addr| unsafe { NonZeroUsize::new_unchecked(addr.get() | Self::TAG_47) }),
        };
        validate_eq!(iter.kind(), node::Kind::NODE_47);
        iter
    }

    #[inline]
    pub(super) fn new_256(node_256: node_256::KeyIter) -> Self {
        let iter = Self { node_256 };
        validate_eq!(iter.kind(), node::Kind::NODE_256);
        iter
    }

    #[inline]
    unsafe fn as_node_15_unchecked(&self) -> NonNull<linear::KeyIter<15>> {
        validate_eq!(self.kind(), node::Kind::NODE_15);
        unsafe {
            self.node_15.map_addr(|addr| {
                validate_eq!(addr.get() & Self::TAG_15, Self::TAG_15);
                NonZeroUsize::new_unchecked(addr.get() ^ Self::TAG_15)
            })
        }
    }

    #[inline]
    unsafe fn as_node_47_unchecked(&self) -> NonNull<linear::KeyIter<47>> {
        validate_eq!(self.kind(), node::Kind::NODE_47);
        unsafe {
            self.node_47.map_addr(|addr| {
                validate_eq!(addr.get() & Self::TAG_47, Self::TAG_47);
                NonZeroUsize::new_unchecked(addr.get() ^ Self::TAG_47)
            })
        }
    }
}

impl Iterator for KeyIter {
    type Item = KeyIndex;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let kind = self.kind();
        if kind == node::Kind::NODE_256 {
            unsafe { &mut self.node_256 }
                .next()
                .map(|key| KeyIndex { key, index: key })
        } else if kind == node::Kind::NODE_3 {
            unsafe { &mut self.node_3 }.next()
        } else if kind == node::Kind::NODE_15 {
            unsafe { self.as_node_15_unchecked().as_mut() }.next()
        } else {
            validate_eq!(kind, node::Kind::NODE_47);
            unsafe { self.as_node_47_unchecked().as_mut() }.next()
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let kind = self.kind();
        if kind == node::Kind::NODE_256 {
            unsafe { &self.node_256 }.size_hint()
        } else if kind == node::Kind::NODE_3 {
            unsafe { &self.node_3 }.size_hint()
        } else if kind == node::Kind::NODE_15 {
            unsafe { self.as_node_15_unchecked().as_ref() }.size_hint()
        } else {
            validate_eq!(kind, node::Kind::NODE_47);
            unsafe { self.as_node_47_unchecked().as_ref() }.size_hint()
        }
    }
}

impl DoubleEndedIterator for KeyIter {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        let kind = self.kind();
        if kind == node::Kind::NODE_256 {
            unsafe { &mut self.node_256 }
                .next_back()
                .map(|key| KeyIndex { key, index: key })
        } else if kind == node::Kind::NODE_3 {
            unsafe { &mut self.node_3 }.next_back()
        } else if kind == node::Kind::NODE_15 {
            unsafe { self.as_node_15_unchecked().as_mut() }.next_back()
        } else {
            validate_eq!(kind, node::Kind::NODE_47);
            unsafe { self.as_node_47_unchecked().as_mut() }.next_back()
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

impl Drop for KeyIter {
    fn drop(&mut self) {
        let kind = self.kind();

        if kind == node::Kind::NODE_15 {
            drop(unsafe { Box::from_raw(self.as_node_15_unchecked().as_ptr()) });
        } else if kind == node::Kind::NODE_47 {
            drop(unsafe { Box::from_raw(self.as_node_47_unchecked().as_ptr()) });
        }
    }
}

pub(crate) trait Lower: Copy + Default {
    fn get(self) -> u8;
    fn check(self, byte: u8) -> bool;
}

pub(crate) trait Upper: Copy + Default {
    fn get(self) -> u8;
    fn check(self, byte: u8) -> bool;
}

impl Lower for Unbound {
    #[inline]
    fn get(self) -> u8 {
        0
    }
    #[inline]
    fn check(self, _byte: u8) -> bool {
        false
    }
}

impl Upper for Unbound {
    #[inline]
    fn get(self) -> u8 {
        255
    }
    #[inline]
    fn check(self, _byte: u8) -> bool {
        false
    }
}

impl Lower for Option<u8> {
    #[inline]
    fn get(self) -> u8 {
        self.unwrap_or(0)
    }
    #[inline]
    fn check(self, byte: u8) -> bool {
        self == Some(byte)
    }
}

impl Upper for Option<u8> {
    #[inline]
    fn get(self) -> u8 {
        self.unwrap_or(255)
    }
    #[inline]
    fn check(self, byte: u8) -> bool {
        self == Some(byte)
    }
}
