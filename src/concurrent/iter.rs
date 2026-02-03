use core::marker::PhantomData;
use core::ops::ControlFlow;

use ribbit::Atomic;

use crate::concurrent::smr;
use crate::concurrent::Key;
use crate::concurrent::Value;
use crate::raw;
use crate::raw::key;
use crate::raw::Edge;

pub struct Prefix<'k, 'v, 'g, K: Key, V, R, G = smr::Epoch> {
    guard: G,
    root: &'g Atomic<Edge<K::Edge>>,
    prefix: K::Read<'k>,
    range: R,
    value: PhantomData<&'v V>,
}

impl<'k, 'v, 'g, K, V, R, G> Prefix<'k, 'v, 'g, K, V, R, G>
where
    K: Key,
    V: Value<'v>,
    R: crate::raw::iter::Range<'k, K>,
    G: smr::Guard<'v, V>,
{
    pub(super) unsafe fn new(
        guard: G,
        root: &'g Atomic<Edge<K::Edge>>,
        prefix: K::Read<'k>,
        range: R,
    ) -> Prefix<'k, 'v, 'g, K, V, R, G> {
        Prefix {
            root,
            prefix,
            guard,
            range,
            value: PhantomData,
        }
    }
}

impl<'k, 'v, 'g, K, V, R, G> Prefix<'k, 'v, 'g, K, V, R, G>
where
    K: Key,
    V: Value<'v>,
    R: crate::raw::iter::Range<'k, K>,
    G: smr::Guard<'v, V>,
{
    #[inline]
    pub fn entries<const REVERSE: bool>(&self) -> EntryIter<'k, 'v, 'g, '_, REVERSE, K, V, R, G> {
        EntryIter {
            _guard: &self.guard,
            iter: unsafe {
                raw::iter::RangeIter::new_unchecked(self.root, self.prefix, self.range.clone())
            },
            value: PhantomData,
        }
    }

    #[inline]
    pub fn values<const REVERSE: bool>(&self) -> ValueIter<'k, 'v, 'g, '_, REVERSE, K, V, R, G> {
        ValueIter {
            _guard: &self.guard,
            iter: unsafe {
                raw::iter::RangeIter::new_unchecked(self.root, self.prefix, self.range.clone())
            },
            value: PhantomData,
        }
    }
}

/// Iterator over keys and values
pub struct EntryIter<
    'k,
    'v,
    'g,
    'l,
    const REVERSE: bool,
    K: Key,
    V: Value<'v>,
    R: raw::iter::Range<'k, K>,
    G,
> {
    _guard: &'l G,
    iter: crate::raw::iter::RangeIter<'k, 'g, REVERSE, K, R, K::Write>,
    value: PhantomData<&'v V>,
}

impl<'k, 'v, 'g, 'l, const REVERSE: bool, K, V, R, G> EntryIter<'k, 'v, 'g, 'l, REVERSE, K, V, R, G>
where
    K: Key,
    V: Value<'v>,
    R: crate::raw::iter::Range<'k, K>,
    G: smr::Guard<'v, V>,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(K::Borrow<'_>, V::Borrow<'v>)> {
        self.iter.lend().map(|(key, value)| {
            (unsafe { K::borrow_writer_unchecked(key) }, unsafe {
                V::borrow_from_raw(value)
            })
        })
    }

    #[inline]
    pub fn for_each<F: FnMut(K::Borrow<'_>, V::Borrow<'v>) -> ControlFlow<()>>(self, mut apply: F) {
        self.iter.for_each(|key, value| {
            apply(unsafe { K::borrow_writer_unchecked(key) }, unsafe {
                V::borrow_from_raw(value)
            })
        })
    }

    #[inline]
    pub fn for_each_raw<F: FnMut(&K::Write, u64) -> ControlFlow<()>>(self, apply: F) {
        self.iter.for_each(apply)
    }
}

impl<'k, 'v, 'g, 'l, const REVERSE: bool, K, V, R, G> Iterator
    for EntryIter<'k, 'v, 'g, 'l, REVERSE, K, V, R, G>
where
    K: Key,
    V: Value<'v>,
    R: crate::raw::iter::Range<'k, K>,
    G: smr::Guard<'v, V>,
{
    type Item = (K, V::Borrow<'v>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.iter.lend().map(|(key, value)| {
            (unsafe { K::from_writer_unchecked(key.clone()) }, unsafe {
                V::borrow_from_raw(value)
            })
        })
    }
}

/// Iterator over values only
pub struct ValueIter<
    'k,
    'v,
    'g,
    'l,
    const REVERSE: bool,
    K: Key,
    V: Value<'v>,
    R: raw::iter::Range<'k, K>,
    G,
> {
    _guard: &'l G,
    iter: crate::raw::iter::RangeIter<'k, 'g, REVERSE, K, R, key::Ignore<K::Edge>>,
    value: PhantomData<&'v V>,
}

impl<'k, 'v, 'g, 'l, const REVERSE: bool, K, V, R, G> ValueIter<'k, 'v, 'g, 'l, REVERSE, K, V, R, G>
where
    K: Key,
    V: Value<'v>,
    R: crate::raw::iter::Range<'k, K>,
    G: smr::Guard<'v, V>,
{
    #[inline]
    pub fn lend(&mut self) -> Option<V::Borrow<'v>> {
        self.iter
            .lend()
            .map(|(_, value)| unsafe { V::borrow_from_raw(value) })
    }

    #[inline]
    pub fn for_each<F: FnMut(V::Borrow<'v>) -> ControlFlow<()>>(self, mut apply: F) {
        self.iter
            .for_each(|_, value| apply(unsafe { V::borrow_from_raw(value) }))
    }
}

impl<'k, 'v, 'g, 'l, const REVERSE: bool, K, V, R, G> Iterator
    for ValueIter<'k, 'v, 'g, 'l, REVERSE, K, V, R, G>
where
    K: Key,
    V: Value<'v>,
    R: crate::raw::iter::Range<'k, K>,
    G: smr::Guard<'v, V>,
{
    type Item = V::Borrow<'v>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.lend()
    }
}
