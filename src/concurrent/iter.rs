use core::marker::PhantomData;
use core::ops::ControlFlow;

use ribbit::Atomic;

use crate::concurrent::smr;
use crate::concurrent::Key;
use crate::concurrent::Value;
use crate::raw;
use crate::raw::Edge;
use crate::sequential;

pub struct Prefix<'k, 'g, K: Key, V, R, G = smr::Epoch> {
    inner: crate::sequential::Prefix<'k, 'g, K, V, R>,
    _guard: G,
}

impl<'k, 'g, K, V, R, G> Prefix<'k, 'g, K, V, R, G>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<'k, K>,
    G: smr::Guard<V>,
{
    #[inline]
    pub(super) unsafe fn new(
        guard: G,
        root: &'g Atomic<Edge<K::Edge>>,
        prefix: K::Read<'k>,
        range: R,
    ) -> Prefix<'k, 'g, K, V, R, G> {
        Prefix {
            _guard: guard,
            inner: sequential::Prefix::new(root, prefix, range),
        }
    }
}

impl<'k, 'g, K, V, R, G> Prefix<'k, 'g, K, V, R, G>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<'k, K>,
    G: smr::Guard<V>,
{
    #[inline]
    pub fn entries<const REVERSE: bool>(&self) -> EntryIter<'k, '_, REVERSE, K, V, R, G> {
        EntryIter {
            inner: self.inner.entries::<REVERSE>(),
            _guard: PhantomData,
        }
    }

    #[inline]
    pub fn values<const REVERSE: bool>(&self) -> ValueIter<'k, '_, REVERSE, K, V, R, G> {
        ValueIter {
            inner: self.inner.values::<REVERSE>(),
            _guard: PhantomData,
        }
    }
}

/// Iterator over keys and values
pub struct EntryIter<'k, 'l, const REVERSE: bool, K: Key, V: Value, R: raw::iter::Range<'k, K>, G> {
    inner: sequential::EntryIter<'k, 'l, REVERSE, K, V, R>,
    _guard: PhantomData<&'l G>,
}

impl<'k, 'l, const REVERSE: bool, K, V, R, G> EntryIter<'k, 'l, REVERSE, K, V, R, G>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<'k, K>,
    G: smr::Guard<V>,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(K::Borrow<'_>, V::Borrow<'l>)> {
        self.inner.lend()
    }

    #[inline]
    pub fn for_each<F: FnMut((K::Borrow<'_>, V::Borrow<'l>)) -> ControlFlow<()>>(self, apply: F) {
        self.inner.for_each(apply)
    }
}

impl<'k, 'l, const REVERSE: bool, K, V, R, G> Iterator for EntryIter<'k, 'l, REVERSE, K, V, R, G>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<'k, K>,
    G: smr::Guard<V>,
{
    type Item = (K, V::Borrow<'l>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

/// Iterator over values only
pub struct ValueIter<'k, 'l, const REVERSE: bool, K: Key, V: Value, R: raw::iter::Range<'k, K>, G> {
    inner: sequential::ValueIter<'k, 'l, REVERSE, K, V, R>,
    _guard: PhantomData<&'l G>,
}

impl<'k, 'l, const REVERSE: bool, K, V, R, G> ValueIter<'k, 'l, REVERSE, K, V, R, G>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<'k, K>,
    G: smr::Guard<V>,
{
    #[inline]
    pub fn for_each<F: FnMut(V::Borrow<'l>) -> ControlFlow<()>>(self, apply: F) {
        self.inner.for_each(apply)
    }
}

impl<'k, 'l, const REVERSE: bool, K, V, R, G> Iterator for ValueIter<'k, 'l, REVERSE, K, V, R, G>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<'k, K>,
    G: smr::Guard<V>,
{
    type Item = V::Borrow<'l>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}
