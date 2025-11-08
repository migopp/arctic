use ribbit::atomic::Atomic128;

use crate::concurrent::cursor;
use crate::concurrent::hazard;
use crate::concurrent::Value;
use crate::iter::Order;
use crate::key;
use crate::key::Read as _;
use crate::raw;
use crate::raw::Edge;
use crate::Key;

/// Guard all nodes and values below this prefix from memory reclamation.
pub struct PrefixGuard<'g, 'l, K: Key, V: Value, R> {
    guard: hazard::PrefixGuard<'g, 'l, V>,
    root: &'g Atomic128<Edge<()>>,
    prefix: K::Read<'l>,
    range: R,
}

impl<'g, 'l, K, V, R> PrefixGuard<'g, 'l, K, V, R>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<K::Read<'l>>,
{
    pub(super) fn new<H>(
        cursor: cursor::Prefix<'g, 'l, K::Read<'l>, (), V, H>,
        range: R,
    ) -> PrefixGuard<'g, 'l, K, V, R>
    where
        K: Key,
        V: Value,
        H: cursor::path::History<'g, K::Read<'l>, ()>,
    {
        let prefix = cursor.prefix();
        let range = range.skip(prefix.bits());
        PrefixGuard {
            root: cursor.edge(),
            prefix,
            guard: cursor.into_guard().guard_prefix(),
            range,
        }
    }
}

impl<'g, 'l, K, V, R> PrefixGuard<'g, 'l, K, V, R>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<K::Read<'l>>,
{
    #[inline]
    #[expect(clippy::type_complexity)]
    pub fn entries<O>(&self) -> EntryIter<'g, 'l, '_, K, V, R, O>
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
    #[expect(clippy::type_complexity)]
    pub fn values<O>(&self) -> ValueIter<'g, 'l, '_, K, V, R, O>
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
pub struct EntryIter<'g, 'l, 'guard, K: Key, V: Value, R: raw::iter::Range<K::Read<'l>>, O> {
    guard: &'guard hazard::PrefixGuard<'g, 'l, V>,
    iter: crate::raw::iter::RangeIter<'g, K::Read<'l>, K::Write, (), R, O>,
}

impl<'g, 'l, 'guard, K, V, R, O> EntryIter<'g, 'l, 'guard, K, V, R, O>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<K::Read<'l>>,
    O: Order,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(K::Borrow<'_>, V::Borrow<'guard>)> {
        self.iter.lend().map(|(key, value)| {
            (unsafe { K::borrow_writer_unchecked(key) }, unsafe {
                V::guard_borrow(self.guard, value)
            })
        })
    }

    #[inline]
    pub fn for_each<F: FnMut(K::Borrow<'_>, V::Borrow<'guard>)>(self, mut apply: F) {
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

impl<'g, 'l, 'guard, K, V, R, O> Iterator for EntryIter<'g, 'l, 'guard, K, V, R, O>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<K::Read<'l>>,
    O: Order,
{
    type Item = (K, V::Borrow<'guard>);

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
pub struct ValueIter<'g, 'l, 'guard, K: Key, V: Value, R: raw::iter::Range<K::Read<'l>>, O> {
    guard: &'guard hazard::PrefixGuard<'g, 'l, V>,
    iter: crate::raw::iter::RangeIter<'g, K::Read<'l>, key::Ignore, (), R, O>,
}

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
            .map(|(key::Ignore, value)| unsafe { V::guard_borrow(self.guard, value) })
    }

    #[inline]
    pub fn for_each<F: FnMut(V::Borrow<'guard>)>(self, mut apply: F) {
        self.iter
            .for_each(|key::Ignore, value| apply(unsafe { V::guard_borrow(self.guard, value) }))
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
