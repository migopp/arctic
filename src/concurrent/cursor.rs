use ribbit::Atomic;

use crate::concurrent::hazard;
use crate::concurrent::hazard::Prefix as _;
use crate::concurrent::Value;
use crate::raw;
pub(super) use crate::raw::cursor::path;
use crate::raw::Edge;
use crate::stat;
use crate::Key;

/// Tree traversal state.
pub(super) struct Point<'k, 'g, 'l, K: Key, V: Value, H> {
    /// SMR guard protecting allocations that overlap with `key`
    guard: hazard::guard::Traverse<'g, 'l, K::Prefix, V>,

    raw: crate::raw::cursor::Point<'k, 'g, K, H>,
}

impl<'k, 'g, 'l, K, V, H> Point<'k, 'g, 'l, K, V, H>
where
    K: Key,
    V: Value,
    H: path::History<'k, 'g, K>,
{
    #[inline]
    pub(super) fn new(
        smr: &'l mut hazard::Local<'g, K::Prefix, V>,
        root: &'g Atomic<Edge<K::Edge>>,
        key: K::Read<'k>,
    ) -> Point<'k, 'g, 'l, K, V, H>
    where
        K: Key,
        V: Value,
        H: path::History<'k, 'g, K>,
    {
        Point {
            guard: smr.guard(
                #[cfg(not(feature = "smr-epoch"))]
                K::hazard(key),
            ),
            raw: unsafe { raw::cursor::Point::new(root, key) },
        }
    }

    #[inline]
    pub(super) fn edge(&self) -> &'g Atomic<Edge<K::Edge>> {
        self.raw.edge()
    }

    #[inline]
    pub(super) unsafe fn retire(&mut self, edge: ribbit::Packed<Edge<K::Edge>>) {
        unsafe { self.guard.retire(self.raw.bits(), edge) }
    }

    #[inline]
    pub(super) fn into_guard(self) -> hazard::guard::Traverse<'g, 'l, K::Prefix, V> {
        self.guard
    }

    #[inline]
    pub(super) fn traverse_exact(&mut self) -> Option<Result<ribbit::Packed<Edge<K::Edge>>, ()>> {
        self.raw.traverse_exact()
    }

    #[inline]
    pub(super) fn traverse_or_insert(&mut self) -> raw::cursor::Insert<K::Edge> {
        self.raw.traverse_or_insert()
    }

    #[cold]
    pub(super) fn freeze(&mut self) -> Result<(), H::PopError> {
        if let Some(edge) = self.raw.freeze()? {
            unsafe {
                self.retire(edge);
            }
        }

        Ok(())
    }
}

impl<'k, 'g, 'l, K, V> Point<'k, 'g, 'l, K, V, path::Discard>
where
    K: Key,
    V: Value,
{
    #[inline]
    pub(super) fn get(
        smr: &'l mut hazard::Local<'g, K::Prefix, V>,
        root: &'g Atomic<Edge<K::Edge>>,
        key: K::Read<'k>,
    ) -> Option<V::SharedGuard<'g, 'l, K::Prefix>> {
        let guard = smr.guard(
            #[cfg(not(feature = "smr-epoch"))]
            K::hazard(key),
        );
        let value = unsafe { crate::raw::cursor::Point::<K, _>::get(root, key)? };
        Some(unsafe { V::guard_shared(guard, value) })
    }
}

pub(super) struct Prefix<'k, 'g, 'l, K: Key, V: Value, H> {
    /// SMR guard protecting allocations that overlap with `key`
    guard: hazard::guard::Traverse<'g, 'l, K::Prefix, V>,

    raw: crate::raw::cursor::Prefix<'k, 'g, K, H>,
}

impl<'k, 'g, 'l, K, V, H> Prefix<'k, 'g, 'l, K, V, H>
where
    K: Key,
    V: Value,
    H: path::History<'k, 'g, K>,
{
    pub(super) fn new(
        smr: &'l mut hazard::Local<'g, K::Prefix, V>,
        root: &'g Atomic<Edge<K::Edge>>,
        prefix: K::Read<'k>,
    ) -> Option<Self> {
        let guard = smr.guard(
            #[cfg(not(feature = "smr-epoch"))]
            K::hazard(prefix),
        );
        Some(Self {
            guard,
            raw: unsafe { crate::raw::cursor::Prefix::new(root, prefix) }?,
        })
    }

    pub(super) fn new_root(
        smr: &'l mut hazard::Local<'g, K::Prefix, V>,
        root: &'g Atomic<Edge<K::Edge>>,
    ) -> Self {
        Self {
            guard: smr.guard(
                #[cfg(not(feature = "smr-epoch"))]
                ribbit::Packed::<K::Prefix>::HAZARD_ROOT,
            ),
            raw: unsafe { crate::raw::cursor::Prefix::new_root(root) },
        }
    }

    pub(super) fn prefix(&self) -> K::Read<'k> {
        self.raw.prefix()
    }

    pub(super) fn edge(&self) -> &'g Atomic<Edge<K::Edge>> {
        self.raw.edge()
    }

    #[inline]
    pub(super) fn into_guard(self) -> hazard::guard::Traverse<'g, 'l, K::Prefix, V> {
        self.guard
    }

    #[expect(unused)]
    pub(super) fn traverse(&mut self) -> Option<ribbit::Packed<Edge<K::Edge>>> {
        self.raw.traverse()
    }

    pub(super) fn wait_for_scan(
        &mut self,
        counter: stat::Counter,
    ) -> Result<ribbit::Packed<Edge<K::Edge>>, ()> {
        self.raw.wait_for_scan(counter)
    }
}

impl<'k, 'g, 'l, K, V> Prefix<'k, 'g, 'l, K, V, path::Hybrid<'k, 'g, K>>
where
    K: Key,
    V: Value,
{
    #[cold]
    pub(super) fn freeze(&mut self) -> Option<ribbit::Packed<Edge<K::Edge>>> {
        self.raw.freeze()
    }
}
