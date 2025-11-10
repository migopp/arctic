use ribbit::atomic::Atomic128;

use crate::concurrent::cursor;
use crate::concurrent::hazard;
use crate::concurrent::Key;
use crate::concurrent::Value;
use crate::iter::Order;
use crate::raw;
use crate::raw::key;
use crate::raw::key::Read as _;
use crate::raw::Edge;

/// Guard all nodes and values below this prefix from memory reclamation.
pub struct PrefixGuard<'g, 'l, 'k, K: Key, V: Value, R> {
    guard: hazard::PrefixGuard<'g, 'l, V>,
    root: &'g Atomic128<Edge<K::Edge>>,
    prefix: K::Read<'k>,
    range: R,
}

#[expect(private_bounds)]
impl<'g, 'l, 'k, K, V, R> PrefixGuard<'g, 'l, 'k, K, V, R>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<K::Read<'k>>,
{
    pub(super) fn new<H>(
        cursor: cursor::Prefix<'g, 'l, 'k, K, V, H>,
        range: R,
    ) -> PrefixGuard<'g, 'l, 'k, K, V, R>
    where
        K: Key,
        V: Value,
        H: cursor::path::History<'g, 'k, K>,
    {
        let prefix = cursor.prefix();
        let range = range.suffix(prefix.bits());
        PrefixGuard {
            root: cursor.edge(),
            prefix,
            guard: cursor.into_guard().guard_prefix(),
            range,
        }
    }
}

#[expect(private_bounds)]
impl<'g, 'l, 'k, K, V, R> PrefixGuard<'g, 'l, 'k, K, V, R>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<K::Read<'k>>,
{
    #[inline]
    pub fn entries<O>(&self) -> EntryIter<'g, '_, 'k, K, V, R, O>
    where
        O: Order,
    {
        EntryIter {
            guard: &self.guard,
            iter: unsafe {
                raw::iter::RangeIter::new_unchecked(self.root, self.prefix, self.range.clone())
            },
        }
    }

    #[inline]
    pub fn values<O>(&self) -> ValueIter<'g, '_, 'k, K, V, R, O>
    where
        O: Order,
    {
        ValueIter {
            guard: &self.guard,
            iter: unsafe {
                raw::iter::RangeIter::new_unchecked(self.root, self.prefix, self.range.clone())
            },
        }
    }

    pub(crate) fn guard_value(self) -> V::LinearizableGuard<'g, 'l> {
        unsafe { V::downgrade_guard(self.guard) }
    }
}

/// Iterator over keys and values
#[expect(private_bounds)]
pub struct EntryIter<'g, 'l, 'k, K: Key, V: Value, R: raw::iter::Range<K::Read<'k>>, O> {
    guard: &'l hazard::PrefixGuard<'g, 'l, V>,
    iter: crate::raw::iter::RangeIter<'g, K::Read<'k>, K::Write, K::Edge, R, O>,
}

#[expect(private_bounds)]
impl<'g, 'l, 'k, K, V, R, O> EntryIter<'g, 'l, 'k, K, V, R, O>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<K::Read<'k>>,
    O: Order,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(<K as Key>::Borrow<'_>, V::Borrow<'l>)> {
        self.iter.lend().map(|(key, value)| {
            (unsafe { K::borrow_writer_unchecked(key) }, unsafe {
                V::guard_borrow(self.guard, value)
            })
        })
    }

    #[inline]
    pub fn for_each<F: FnMut(<K as Key>::Borrow<'_>, V::Borrow<'l>)>(self, mut apply: F) {
        self.iter.for_each(|key, value| {
            apply(unsafe { K::borrow_writer_unchecked(key) }, unsafe {
                V::guard_borrow(self.guard, value)
            })
        })
    }

    #[inline]
    pub fn for_each_raw<F: FnMut(&K::Write, u64)>(self, apply: F) {
        self.iter.for_each(apply)
    }
}

impl<'g, 'l, 'k, K, V, R, O> Iterator for EntryIter<'g, 'l, 'k, K, V, R, O>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<K::Read<'k>>,
    O: Order,
{
    type Item = (K, V::Borrow<'l>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.iter.lend().map(|(key, value)| {
            (unsafe { K::from_writer_unchecked(key.clone()) }, unsafe {
                V::guard_borrow(self.guard, value)
            })
        })
    }
}

/// Iterator over values only
#[expect(private_bounds)]
pub struct ValueIter<'g, 'l, 'guard, K: Key, V: Value, R: raw::iter::Range<K::Read<'l>>, O> {
    guard: &'guard hazard::PrefixGuard<'g, 'l, V>,
    iter: crate::raw::iter::RangeIter<'g, K::Read<'l>, key::Ignore<K::Edge>, K::Edge, R, O>,
}

#[expect(private_bounds)]
impl<'g, 'l, 'guard, K, V, R, O> ValueIter<'g, 'l, 'guard, K, V, R, O>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<K::Read<'l>>,
    O: Order,
{
    #[inline]
    pub fn lend(&mut self) -> Option<V::Borrow<'guard>> {
        self.iter
            .lend()
            .map(|(_, value)| unsafe { V::guard_borrow(self.guard, value) })
    }

    #[inline]
    pub fn for_each<F: FnMut(V::Borrow<'guard>)>(self, mut apply: F) {
        self.iter
            .for_each(|_, value| apply(unsafe { V::guard_borrow(self.guard, value) }))
    }
}

impl<'g, 'l, 'guard, K, V, R, O> Iterator for ValueIter<'g, 'l, 'guard, K, V, R, O>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<K::Read<'l>>,
    O: Order,
{
    type Item = V::Borrow<'guard>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.lend()
    }
}
