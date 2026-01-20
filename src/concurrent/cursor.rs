use ribbit::Atomic;

use crate::concurrent::hazard;
use crate::concurrent::Value;
use crate::raw;
pub(super) use crate::raw::cursor::path;
use crate::raw::Edge;
use crate::raw::Smo;
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
    pub(super) fn bits(&self) -> usize {
        self.raw.bits()
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
    pub(super) fn traverse_get(self) -> Option<V::SharedGuard<'g, 'l, K::Prefix>> {
        let value = unsafe { self.raw.traverse_get() }?;
        Some(unsafe { V::guard_shared(self.guard, value) })
    }

    #[inline]
    pub(super) fn traverse_update(&mut self) -> Option<Result<ribbit::Packed<Edge<K::Edge>>, ()>> {
        self.raw.traverse_update()
    }

    #[inline]
    pub(super) fn traverse_upsert(
        &mut self,
        value: u64,
    ) -> Result<
        (
            Smo,
            ribbit::Packed<Edge<K::Edge>>,
            ribbit::Packed<Edge<K::Edge>>,
        ),
        (),
    > {
        self.raw.traverse_upsert(value)
    }

    #[inline]
    pub(super) fn traverse_prefix(&mut self) -> Option<ribbit::Packed<Edge<K::Edge>>> {
        self.raw.traverse_prefix()
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
