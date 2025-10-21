use core::marker::PhantomData;
use core::ptr::NonNull;

use ribbit::atomic::Atomic128;

use crate::Edge;

#[repr(transparent)]
#[derive(Copy, Clone)]
pub(crate) struct SortedIter<'a, K, V>(Iter<'a, K, V>);

impl<'a, K, V> SortedIter<'a, K, V> {
    /// # SAFETY
    ///
    /// Caller must guarantee all indices produced by `keys` are < `edges.len()`.
    pub(super) unsafe fn new(keys: K, edges: &[Atomic128<Edge<V>>]) -> Self {
        Self(Iter::new(keys, edges))
    }
}

impl<'a, K, V> Iterator for SortedIter<'a, K, V>
where
    K: Iterator<Item = (u8, u8)>,
{
    type Item = (u8, &'a Atomic128<Edge<V>>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (key, index) = self.0.keys.next()?;

        #[cfg(feature = "validate")]
        validate!(
            (index as u16) < self.0.len,
            "index is {} but len is {}",
            index,
            self.0.len,
        );

        let edge = unsafe { self.0.edges.add(index as usize).as_ref() };
        Some((key, edge))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.keys.size_hint()
    }
}

impl<'a, K, V> DoubleEndedIterator for SortedIter<'a, K, V>
where
    K: DoubleEndedIterator<Item = (u8, u8)>,
{
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        let (key, index) = self.0.keys.next_back()?;

        #[cfg(feature = "validate")]
        validate!(
            (index as u16) < self.0.len,
            "index is {} but len is {}",
            index,
            self.0.len,
        );

        let edge = unsafe { self.0.edges.add(index as usize).as_ref() };
        Some((key, edge))
    }
}

impl<'a, K, V> ExactSizeIterator for SortedIter<'a, K, V>
where
    K: ExactSizeIterator<Item = (u8, u8)>,
{
    #[inline]
    fn len(&self) -> usize {
        let (lower, upper) = self.size_hint();
        validate_eq!(upper, Some(lower));
        lower
    }
}

#[repr(transparent)]
#[derive(Copy, Clone)]
pub(crate) struct UnsortedIter<'a, K, V>(Iter<'a, K, V>);

impl<'a, K, V> UnsortedIter<'a, K, V> {
    /// # SAFETY
    ///
    /// Caller must guarantee `keys` produces at most `edges.len()` keys.
    pub(super) unsafe fn new(keys: K, edges: &[Atomic128<Edge<V>>]) -> Self {
        Self(Iter::new(keys, edges))
    }
}

impl<'a, K, V> Iterator for UnsortedIter<'a, K, V>
where
    K: Iterator<Item = u8>,
{
    type Item = (u8, &'a Atomic128<Edge<V>>);

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

impl<'a, K, V> ExactSizeIterator for UnsortedIter<'a, K, V>
where
    K: ExactSizeIterator<Item = u8>,
{
    #[inline]
    fn len(&self) -> usize {
        let (lower, upper) = self.size_hint();
        validate_eq!(upper, Some(lower));
        lower
    }
}

#[derive(Copy, Clone)]
struct Iter<'a, K, V> {
    keys: K,
    edges: NonNull<Atomic128<Edge<V>>>,

    #[cfg(feature = "validate")]
    len: u16,

    _slice: PhantomData<&'a [Atomic128<Edge<V>>]>,
}

impl<'a, K, V> Iter<'a, K, V> {
    #[inline]
    unsafe fn new(keys: K, edges: &[Atomic128<Edge<V>>]) -> Self {
        Self {
            keys,
            edges: NonNull::from(edges).cast(),

            #[cfg(feature = "validate")]
            len: edges.len() as u16,

            _slice: PhantomData,
        }
    }
}
