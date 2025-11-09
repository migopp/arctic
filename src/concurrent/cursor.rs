use ribbit::atomic::Atomic128;

use crate::concurrent::hazard;
use crate::concurrent::Value;
use crate::raw;
pub(super) use crate::raw::cursor::path;
use crate::raw::Edge;
use crate::raw::Op;
use crate::stat;
use crate::Key;

/// Tree traversal state.
pub(super) struct Point<'g, 'l, 'k, K: Key, V: Value, H> {
    /// SMR guard protecting allocations that overlap with `key`
    guard: hazard::TraverseGuard<'g, 'l, V>,

    raw: crate::raw::cursor::Point<'g, 'k, K, H>,
}

impl<'g, 'l, 'k, K, V, H> Point<'g, 'l, 'k, K, V, H>
where
    K: Key,
    V: Value,
    H: path::History<'g, 'k, K>,
{
    #[inline]
    pub(super) fn new(
        smr: &'l mut hazard::Local<'g, V>,
        root: &'g Atomic128<Edge<K::Edge>>,
        key: K::Read<'k>,
    ) -> Point<'g, 'l, 'k, K, V, H>
    where
        K: Key,
        V: Value,
        H: path::History<'g, 'k, K>,
    {
        Point {
            guard: smr.guard(K::hazard(key)),
            raw: unsafe { raw::cursor::Point::new(root, key) },
        }
    }

    #[inline]
    pub(super) fn edge(&self) -> &'g Atomic128<Edge<K::Edge>> {
        self.raw.edge()
    }

    #[inline]
    pub(super) unsafe fn retire(&mut self, edge: ribbit::Packed<Edge<K::Edge>>) {
        unsafe { self.guard.retire(self.raw.bits(), edge) }
    }

    #[inline]
    pub(super) fn into_guard(self) -> hazard::TraverseGuard<'g, 'l, V> {
        self.guard
    }

    #[inline]
    pub(super) fn traverse_exact(&mut self) -> Option<Result<ribbit::Packed<Edge<K::Edge>>, ()>> {
        self.raw.traverse_exact()
    }

    #[inline]
    pub(super) fn traverse_or_insert(
        &mut self,
        value: u64,
    ) -> Result<
        (
            Op,
            ribbit::Packed<Edge<K::Edge>>,
            ribbit::Packed<Edge<K::Edge>>,
        ),
        (),
    > {
        self.raw.traverse_or_insert(value)
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

    #[cold]
    pub(super) fn wait_for_scan(
        &self,
        counter: stat::Counter,
    ) -> Result<ribbit::Packed<Edge<K::Edge>>, ()> {
        self.raw.wait_for_scan(counter)
    }
}

impl<'g, 'l, 'k, K, V> Point<'g, 'l, 'k, K, V, path::Discard>
where
    K: Key,
    V: Value,
{
    #[inline]
    pub(super) fn get(
        smr: &'l mut hazard::Local<'g, V>,
        root: &'g Atomic128<Edge<K::Edge>>,
        key: K::Read<'k>,
    ) -> Option<V::SharedGuard<'g, 'l>> {
        let guard = smr.guard(K::hazard(key));
        let value = unsafe { crate::raw::cursor::Point::<K, _>::get(root, key)? };
        Some(unsafe { V::guard_shared(guard, value) })
    }
}

pub(super) struct Prefix<'g, 'l, 'k, K: Key, V: Value, H> {
    /// SMR guard protecting allocations that overlap with `key`
    guard: hazard::TraverseGuard<'g, 'l, V>,

    raw: crate::raw::cursor::Prefix<'g, 'k, K, H>,
}

impl<'g, 'l, 'k, K, V, H> Prefix<'g, 'l, 'k, K, V, H>
where
    K: Key,
    V: Value,
    H: path::History<'g, 'k, K>,
{
    pub(super) fn new(
        smr: &'l mut hazard::Local<'g, V>,
        root: &'g Atomic128<Edge<K::Edge>>,
        prefix: K::Read<'k>,
    ) -> Option<Self> {
        let guard = smr.guard(K::hazard(prefix));
        Some(Self {
            guard,
            raw: unsafe { crate::raw::cursor::Prefix::new(root, prefix) }?,
        })
    }

    pub(super) fn new_root(
        smr: &'l mut hazard::Local<'g, V>,
        root: &'g Atomic128<Edge<K::Edge>>,
    ) -> Self {
        Self {
            guard: smr.guard(hazard::prefix::Be::HAZARD_ROOT),
            raw: unsafe { crate::raw::cursor::Prefix::new_root(root) },
        }
    }

    pub(super) fn prefix(&self) -> K::Read<'k> {
        self.raw.prefix()
    }

    pub(super) fn edge(&self) -> &'g Atomic128<Edge<K::Edge>> {
        self.raw.edge()
    }

    #[inline]
    pub(super) fn into_guard(self) -> hazard::TraverseGuard<'g, 'l, V> {
        self.guard
    }

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

impl<'g, 'l, 'k, K, V> Prefix<'g, 'l, 'k, K, V, path::Hybrid<'g, 'k, K>>
where
    K: Key,
    V: Value,
{
    #[cold]
    pub(super) fn freeze(&mut self) -> Option<ribbit::Packed<Edge<K::Edge>>> {
        self.raw.freeze()
    }
}
