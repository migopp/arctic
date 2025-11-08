use core::marker::PhantomData;
use core::ptr::NonNull;

use ribbit::atomic::Atomic128;

use crate::raw::Edge;

pub(crate) struct SortedIter<'g, L, U, I, V> {
    lower: L,
    upper: U,
    iter: Iter<'g, I, V>,
}

impl<'g, L, U, I, V> SortedIter<'g, L, U, I, V> {
    /// # SAFETY
    ///
    /// Caller must guarantee all indices produced by `keys` are < `edges.len()`.
    pub(super) unsafe fn new(lower: L, upper: U, keys: I, edges: &'g [Atomic128<Edge<V>>]) -> Self {
        Self {
            lower,
            upper,
            iter: Iter::new(keys, edges),
        }
    }

    pub(crate) fn lower(&self) -> &L {
        &self.lower
    }

    pub(crate) fn upper(&self) -> &U {
        &self.upper
    }
}

impl<'g, L, U, I, V> Iterator for SortedIter<'g, L, U, I, V>
where
    I: Iterator<Item = (u8, u8)>,
{
    type Item = (u8, &'g Atomic128<Edge<V>>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (key, index) = self.iter.keys.next()?;

        #[cfg(feature = "validate")]
        validate!(
            (index as u16) < self.iter.len,
            "index is {} but len is {}",
            index,
            self.iter.len,
        );

        let edge = unsafe { self.iter.edges.add(index as usize).as_ref() };
        Some((key, edge))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.keys.size_hint()
    }
}

impl<'g, L, U, I, V> DoubleEndedIterator for SortedIter<'g, L, U, I, V>
where
    I: DoubleEndedIterator<Item = (u8, u8)>,
{
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        let (key, index) = self.iter.keys.next_back()?;

        #[cfg(feature = "validate")]
        validate!(
            (index as u16) < self.iter.len,
            "index is {} but len is {}",
            index,
            self.iter.len,
        );

        let edge = unsafe { self.iter.edges.add(index as usize).as_ref() };
        Some((key, edge))
    }
}

impl<'g, L, U, I, V> ExactSizeIterator for SortedIter<'g, L, U, I, V>
where
    I: ExactSizeIterator<Item = (u8, u8)>,
{
    #[inline]
    fn len(&self) -> usize {
        let (lower, upper) = self.size_hint();
        validate_eq!(upper, Some(lower));
        lower
    }
}

#[repr(transparent)]
pub(crate) struct UnsortedIter<'g, I, V>(Iter<'g, I, V>);

impl<'g, I, V> UnsortedIter<'g, I, V> {
    /// # SAFETY
    ///
    /// Caller must guarantee `keys` produces at most `edges.len()` keys.
    pub(crate) unsafe fn new(keys: I, edges: &'g [Atomic128<Edge<V>>]) -> Self {
        Self(Iter::new(keys, edges))
    }
}

impl<'g, I, V> Iterator for UnsortedIter<'g, I, V>
where
    I: Iterator<Item = u8>,
{
    type Item = (u8, &'g Atomic128<Edge<V>>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let key = self.0.keys.next()?;

        #[cfg(feature = "validate")]
        {
            validate!(self.0.len > 0);
            self.0.len -= 1;
        }

        let edge = unsafe {
            let edge = self.0.edges.as_ref();
            self.0.edges = self.0.edges.add(1);
            edge
        };
        Some((key, edge))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.keys.size_hint()
    }
}

impl<'g, I, V> ExactSizeIterator for UnsortedIter<'g, I, V>
where
    I: ExactSizeIterator<Item = u8>,
{
    #[inline]
    fn len(&self) -> usize {
        let (lower, upper) = self.size_hint();
        validate_eq!(upper, Some(lower));
        lower
    }
}

struct Iter<'g, I, V> {
    keys: I,
    edges: NonNull<Atomic128<Edge<V>>>,

    #[cfg(feature = "validate")]
    len: u16,

    _slice: PhantomData<&'g [Atomic128<Edge<V>>]>,
}

impl<'g, I, V> Iter<'g, I, V> {
    #[inline]
    fn new(keys: I, edges: &'g [Atomic128<Edge<V>>]) -> Self {
        Self {
            keys,
            edges: NonNull::from(edges).cast(),

            #[cfg(feature = "validate")]
            len: edges.len() as u16,

            _slice: PhantomData,
        }
    }
}

pub(crate) trait Low: Copy + Default {
    const UNBOUND: bool;
    fn get(self) -> u8;
    fn is(self, byte: u8) -> bool;
}

pub(crate) trait High: Copy + Default {
    const UNBOUND: bool;
    fn get(self) -> u8;
    fn is(self, byte: u8) -> bool;
}

impl Low for crate::iter::Unbound {
    const UNBOUND: bool = true;
    #[inline]
    fn get(self) -> u8 {
        0
    }
    #[inline]
    fn is(self, _byte: u8) -> bool {
        true
    }
}

impl High for crate::iter::Unbound {
    const UNBOUND: bool = true;
    #[inline]
    fn get(self) -> u8 {
        255
    }
    #[inline]
    fn is(self, _byte: u8) -> bool {
        true
    }
}

impl Low for Option<u8> {
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

impl High for Option<u8> {
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
