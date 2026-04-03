use core::marker::PhantomData;
use core::ops::ControlFlow;

use crate::Order;
use crate::concurrent::Key;
use crate::concurrent::Value;
use crate::concurrent::smr;
use crate::raw;
use crate::sequential;

pub struct Prefix<'k, 'g, K: Key, V, R, G> {
    inner: sequential::Prefix<'k, 'g, K, V, R>,
    _guard: G,
}

impl<'k, 'g, K, V, R, G> Prefix<'k, 'g, K, V, R, G>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<'k, K>,
    G: smr::Guard<V>,
{
    #[inline]
    pub(super) unsafe fn new(
        guard: G,
        prefix: raw::iter::Prefix<'k, 'g, K, R>,
    ) -> Prefix<'k, 'g, K, V, R, G> {
        Prefix {
            _guard: guard,
            inner: sequential::Prefix::new(prefix),
        }
    }
}

impl<'k, 'g, K, V, R, G> Prefix<'k, 'g, K, V, R, G>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<'k, K>,
    G: smr::Guard<V>,
{
    #[inline]
    pub fn entries<O: Order>(&self) -> EntryIter<'k, '_, K, V, R, O, G> {
        EntryIter {
            inner: self.inner.entries::<O>(),
            _guard: PhantomData,
        }
    }

    #[inline]
    pub fn values<O: Order>(&self) -> ValueIter<'k, '_, K, V, R, O, G> {
        ValueIter {
            inner: self.inner.values::<O>(),
            _guard: PhantomData,
        }
    }
}

/// Iterator over keys and values
pub struct EntryIter<'k, 'l, K: Key, V: Value, R: raw::iter::Range<'k, K>, O, G> {
    inner: sequential::EntryIter<'k, 'l, K, V, R, O>,
    _guard: PhantomData<&'l G>,
}

impl<'k, 'l, K, V, R, O, G> EntryIter<'k, 'l, K, V, R, O, G>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<'k, K>,
    O: Order,
    G: smr::Guard<V>,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(K::Borrow<'_>, V::Borrow<'l>)> {
        self.inner.lend()
    }

    #[inline]
    pub fn for_each_internal<F: FnMut((K::Borrow<'_>, V::Borrow<'l>)) -> ControlFlow<()>>(
        self,
        apply: F,
    ) {
        self.inner.for_each_internal(apply)
    }
}

impl<'k, 'l, K, V, R, O, G> Iterator for EntryIter<'k, 'l, K, V, R, O, G>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<'k, K>,
    O: Order,
    G: smr::Guard<V>,
{
    type Item = (K, V::Borrow<'l>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

/// Iterator over values only
pub struct ValueIter<'k, 'l, K: Key, V: Value, R: raw::iter::Range<'k, K>, O, G> {
    inner: sequential::ValueIter<'k, 'l, K, V, R, O>,
    _guard: PhantomData<&'l G>,
}

impl<'k, 'l, K, V, R, O, G> ValueIter<'k, 'l, K, V, R, O, G>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<'k, K>,
    O: Order,
    G: smr::Guard<V>,
{
    #[inline]
    pub fn for_each_internal<F: FnMut(V::Borrow<'l>) -> ControlFlow<()>>(self, apply: F) {
        self.inner.for_each_internal(apply)
    }
}

impl<'k, 'l, K, V, R, O, G> Iterator for ValueIter<'k, 'l, K, V, R, O, G>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<'k, K>,
    O: Order,
    G: smr::Guard<V>,
{
    type Item = V::Borrow<'l>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}
