use core::ops::ControlFlow;

use ribbit::Atomic;

use crate::concurrent::cursor;
use crate::concurrent::hazard;
use crate::concurrent::Key;
use crate::concurrent::Value;
use crate::raw;
use crate::raw::key;
use crate::raw::key::Read as _;
use crate::raw::Edge;

/// Guard all nodes and values below this prefix from memory reclamation.
pub struct Prefix<'k, 'g, 'l, K: Key, V: Value, R> {
    guard: hazard::guard::Prefix<'g, 'l, K::Prefix, V>,
    root: &'g Atomic<Edge<K::Edge>>,
    prefix: K::Read<'k>,
    range: R,
}

#[expect(private_bounds)]
impl<'k, 'g, 'l, K, V, R> Prefix<'k, 'g, 'l, K, V, R>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<K::Read<'k>>,
{
    pub(super) fn new<H>(
        cursor: cursor::Prefix<'k, 'g, 'l, K, V, H>,
        range: R,
    ) -> Prefix<'k, 'g, 'l, K, V, R>
    where
        K: Key,
        V: Value,
        H: cursor::path::History<'k, 'g, K>,
    {
        let prefix = cursor.prefix();
        let range = range.suffix(prefix.bits());
        Prefix {
            root: cursor.edge(),
            prefix,
            guard: cursor.into_guard().guard_prefix(),
            range,
        }
    }
}

#[expect(private_bounds)]
impl<'k, 'g, 'l, K, V, R> Prefix<'k, 'g, 'l, K, V, R>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<K::Read<'k>>,
{
    #[inline]
    pub fn entries<const REVERSE: bool>(&self) -> EntryIter<'k, 'g, '_, REVERSE, K, V, R> {
        EntryIter {
            guard: &self.guard,
            iter: unsafe {
                raw::iter::RangeIter::new_unchecked(self.root, self.prefix, self.range.clone())
            },
        }
    }

    #[inline]
    pub fn values<const REVERSE: bool>(&self) -> ValueIter<'k, 'g, '_, REVERSE, K, V, R> {
        ValueIter {
            guard: &self.guard,
            iter: unsafe {
                raw::iter::RangeIter::new_unchecked(self.root, self.prefix, self.range.clone())
            },
        }
    }

    pub(crate) fn guard_value(self) -> V::LinearizableGuard<'g, 'l, K::Prefix> {
        unsafe { V::downgrade_guard(self.guard) }
    }
}

/// Iterator over keys and values
#[expect(private_bounds)]
pub struct EntryIter<
    'k,
    'g,
    'l,
    const REVERSE: bool,
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
> {
    guard: &'l hazard::guard::Prefix<'g, 'l, K::Prefix, V>,
    iter: crate::raw::iter::RangeIter<'g, REVERSE, K::Read<'k>, K::Write, K::Edge, R>,
}

#[expect(private_bounds)]
impl<'k, 'g, 'l, const REVERSE: bool, K, V, R> EntryIter<'k, 'g, 'l, REVERSE, K, V, R>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<K::Read<'k>>,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(K::Borrow<'_>, V::Borrow<'l>)> {
        self.iter.lend().map(|(key, value)| {
            (unsafe { K::borrow_writer_unchecked(key) }, unsafe {
                V::guard_borrow(self.guard, value)
            })
        })
    }

    #[inline]
    pub fn for_each<F: FnMut(K::Borrow<'_>, V::Borrow<'l>) -> ControlFlow<()>>(self, mut apply: F) {
        self.iter.for_each(|key, value| {
            apply(unsafe { K::borrow_writer_unchecked(key) }, unsafe {
                V::guard_borrow(self.guard, value)
            })
        })
    }

    #[inline]
    pub fn for_each_raw<F: FnMut(&K::Write, u64) -> ControlFlow<()>>(self, apply: F) {
        self.iter.for_each(apply)
    }
}

impl<'k, 'g, 'l, const REVERSE: bool, K, V, R> Iterator for EntryIter<'k, 'g, 'l, REVERSE, K, V, R>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<K::Read<'k>>,
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
pub struct ValueIter<
    'k,
    'g,
    'l,
    const REVERSE: bool,
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
> {
    guard: &'l hazard::guard::Prefix<'g, 'l, K::Prefix, V>,
    iter: crate::raw::iter::RangeIter<'g, REVERSE, K::Read<'k>, key::Ignore<K::Edge>, K::Edge, R>,
}

#[expect(private_bounds)]
impl<'k, 'g, 'l, const REVERSE: bool, K, V, R> ValueIter<'k, 'g, 'l, REVERSE, K, V, R>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<K::Read<'k>>,
{
    #[inline]
    pub fn lend(&mut self) -> Option<V::Borrow<'l>> {
        self.iter
            .lend()
            .map(|(_, value)| unsafe { V::guard_borrow(self.guard, value) })
    }

    #[inline]
    pub fn for_each<F: FnMut(V::Borrow<'l>) -> ControlFlow<()>>(self, mut apply: F) {
        self.iter
            .for_each(|_, value| apply(unsafe { V::guard_borrow(self.guard, value) }))
    }
}

impl<'k, 'g, 'l, const REVERSE: bool, K, V, R> Iterator for ValueIter<'k, 'g, 'l, REVERSE, K, V, R>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<K::Read<'k>>,
{
    type Item = V::Borrow<'l>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.lend()
    }
}
