use core::marker::PhantomData;
use core::num::NonZeroUsize;
use core::ptr::NonNull;

use ribbit::Atomic;

use crate::raw::iter::Unbound;
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
    node_3: linear::KeyIter<3>,
    node_15: NonNull<linear::KeyIter<15>>,
    node_256: node_256::KeyIter,
    raw: usize,
}

impl KeyIter {
    const TAG_256: usize = 1usize.rotate_right(1);
    const TAG_15: usize = 0b100
        << if cfg!(target_endian = "little") {
            56
        } else {
            0
        };

    pub(crate) const ROOT: Self = Self {
        node_3: linear::KeyIter::new([(0, 0); 3], 1),
    };

    #[inline]
    pub(super) fn from_node_3(node_3: linear::KeyIter<3>) -> Self {
        let iter = Self { node_3 };
        validate_eq!(unsafe { iter.raw } & Self::TAG_15, 0);
        validate_eq!(unsafe { iter.raw } & Self::TAG_256, 0);
        iter
    }

    #[inline]
    pub(super) fn from_node_15(node_15: linear::KeyIter<15>) -> Self {
        let node_15 = NonNull::from(Box::leak(Box::new(node_15))).map_addr(|addr| {
            validate_eq!(addr.get() & Self::TAG_15, 0);
            validate_eq!(addr.get() & Self::TAG_256, 0);
            unsafe { NonZeroUsize::new_unchecked(addr.get() | Self::TAG_15) }
        });
        Self { node_15 }
    }

    #[inline]
    pub(super) fn from_node_256(iter: node_256::KeyIter) -> Self {
        let iter = Self { node_256: iter };
        validate_eq!(unsafe { iter.raw } & Self::TAG_256, Self::TAG_256);
        iter
    }

    fn is_node_256(&self) -> bool {
        (unsafe { self.raw } & Self::TAG_256) > 0
    }

    fn is_node_15(&self) -> bool {
        (unsafe { self.raw } & Self::TAG_15) > 0
    }

    fn as_node_15(&self) -> Option<&linear::KeyIter<15>> {
        self.as_node_15_raw()
            .map(|node_15| unsafe { node_15.as_ref() })
    }

    fn as_node_15_mut(&mut self) -> Option<&mut linear::KeyIter<15>> {
        self.as_node_15_raw()
            .map(|mut node_15| unsafe { node_15.as_mut() })
    }

    fn as_node_15_raw(&self) -> Option<NonNull<linear::KeyIter<15>>> {
        self.is_node_15().then(|| unsafe {
            self.node_15.map_addr(|addr| {
                validate_eq!(addr.get() & Self::TAG_15, Self::TAG_15);
                NonZeroUsize::new_unchecked(addr.get() ^ Self::TAG_15)
            })
        })
    }
}

impl Iterator for KeyIter {
    type Item = (u8, u8);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.is_node_256() {
            unsafe { &mut self.node_256 }.next().map(|key| (key, key))
        } else if let Some(node_15) = self.as_node_15_mut() {
            node_15.next()
        } else {
            unsafe { &mut self.node_3 }.next()
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.is_node_256() {
            unsafe { &self.node_256 }.size_hint()
        } else if let Some(node_15) = self.as_node_15() {
            node_15.size_hint()
        } else {
            unsafe { &self.node_3 }.size_hint()
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
        } else if let Some(node_15) = self.as_node_15_mut() {
            node_15.next_back()
        } else {
            unsafe { &mut self.node_3 }.next_back()
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
        if let Some(node_15) = self.as_node_15_raw() {
            drop(unsafe { Box::from_raw(node_15.as_ptr()) });
        }
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
