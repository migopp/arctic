use core::marker::PhantomData;
use core::ptr::NonNull;

use ribbit::atomic::Atomic128;

use crate::raw::Edge;

#[repr(transparent)]
pub(crate) struct SortedIter<'g, I, V>(Iter<'g, I, V>);

impl<'g, I, V> SortedIter<'g, I, V> {
    /// # SAFETY
    ///
    /// Caller must guarantee all indices produced by `keys` are < `edges.len()`.
    pub(super) unsafe fn new(keys: I, edges: &[Atomic128<Edge<V>>]) -> Self {
        Self(Iter::new(keys, edges))
    }
}

impl<'g, I, V> Iterator for SortedIter<'g, I, V>
where
    I: Iterator<Item = (u8, u8)>,
{
    type Item = (u8, &'g Atomic128<Edge<V>>);

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

impl<'g, I, V> DoubleEndedIterator for SortedIter<'g, I, V>
where
    I: DoubleEndedIterator<Item = (u8, u8)>,
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

impl<'g, I, V> ExactSizeIterator for SortedIter<'g, I, V>
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
    pub(crate) unsafe fn new(keys: I, edges: &[Atomic128<Edge<V>>]) -> Self {
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
    unsafe fn new(keys: I, edges: &[Atomic128<Edge<V>>]) -> Self {
        Self {
            keys,
            edges: NonNull::from(edges).cast(),

            #[cfg(feature = "validate")]
            len: edges.len() as u16,

            _slice: PhantomData,
        }
    }
}
