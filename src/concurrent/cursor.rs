use ribbit::atomic::Atomic128;

use crate::concurrent::hazard;
use crate::concurrent::Value;
use crate::key;
use crate::raw;
pub(super) use crate::raw::cursor::path;
use crate::raw::edge;
use crate::raw::Edge;
use crate::raw::Op;
use crate::stat;

/// Tree traversal state.
pub(super) struct Point<'g, 'l, R, M, V: Value, H> {
    /// SMR guard protecting allocations that overlap with `key`
    guard: hazard::TraverseGuard<'g, 'l, V>,

    raw: crate::raw::cursor::Point<'g, R, M, H>,
}

impl<'g, 'l, R, M, V, H> Point<'g, 'l, R, M, V, H>
where
    R: key::Read<Edge = M>,
    M: edge::Meta,
    V: Value,
    H: path::History<'g, R, M>,
{
    #[inline]
    pub(super) fn new(
        smr: &'l mut hazard::Local<'g, V>,
        root: &'g Atomic128<Edge<M>>,
        key: R,
    ) -> Self {
        Self {
            guard: smr.guard(key.hazard()),
            raw: unsafe { raw::cursor::Point::new(root, key) },
        }
    }

    #[inline]
    pub(super) fn edge(&self) -> &'g Atomic128<Edge<M>> {
        self.raw.edge()
    }

    #[inline]
    pub(super) unsafe fn retire(&mut self, edge: ribbit::Packed<Edge<M>>) {
        unsafe { self.guard.retire(self.raw.bits(), edge) }
    }

    #[inline]
    pub(super) fn into_guard(self) -> hazard::TraverseGuard<'g, 'l, V> {
        self.guard
    }

    #[inline]
    pub(super) fn traverse_exact(&mut self) -> Option<Result<ribbit::Packed<Edge<M>>, ()>> {
        self.raw.traverse_exact()
    }

    #[inline]
    pub(super) fn traverse_or_insert(
        &mut self,
        value: u64,
    ) -> Result<(Op, ribbit::Packed<Edge<M>>, ribbit::Packed<Edge<M>>), ()> {
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
    ) -> Result<ribbit::Packed<Edge<M>>, ()> {
        self.raw.wait_for_scan(counter)
    }
}

impl<'g, 'l, R, M, V> Point<'g, 'l, R, M, V, path::Discard>
where
    R: key::Read<Edge = M>,
    M: edge::Meta,
    V: Value,
{
    #[inline]
    pub(super) fn get(
        smr: &'l mut hazard::Local<'g, V>,
        root: &'g Atomic128<Edge<M>>,
        key: R,
    ) -> Option<V::SharedGuard<'g, 'l>>
    where
        R: key::Read,
        V: Value,
    {
        let guard = smr.guard(key.hazard());
        let value = unsafe { crate::raw::cursor::Point::get(root, key)? };
        Some(unsafe { V::guard_shared(guard, value) })
    }
}

pub(super) struct Prefix<'g, 'l, R, C, V: Value, H> {
    /// SMR guard protecting allocations that overlap with `key`
    guard: hazard::TraverseGuard<'g, 'l, V>,

    raw: crate::raw::cursor::Prefix<'g, R, C, H>,
}

impl<'g, 'l, R, M, V, H> Prefix<'g, 'l, R, M, V, H>
where
    R: key::Read<Edge = M>,
    M: edge::Meta,
    V: Value,
    H: path::History<'g, R, M>,
{
    pub(super) fn new(
        smr: &'l mut hazard::Local<'g, V>,
        root: &'g Atomic128<Edge<M>>,
        prefix: R,
    ) -> Option<Self> {
        let guard = smr.guard(prefix.hazard());
        Some(Self {
            guard,
            raw: unsafe { crate::raw::cursor::Prefix::new(root, prefix) }?,
        })
    }

    pub(super) fn new_root(
        smr: &'l mut hazard::Local<'g, V>,
        root: &'g Atomic128<Edge<M>>,
    ) -> Self {
        Self {
            guard: smr.guard(hazard::prefix::Be::HAZARD_ROOT),
            raw: unsafe { crate::raw::cursor::Prefix::new_root(root) },
        }
    }

    pub(super) fn prefix(&self) -> R {
        self.raw.prefix()
    }

    pub(super) fn edge(&self) -> &'g Atomic128<Edge<M>> {
        self.raw.edge()
    }

    #[inline]
    pub(super) fn into_guard(self) -> hazard::TraverseGuard<'g, 'l, V> {
        self.guard
    }

    pub(super) fn traverse(&mut self) -> Option<ribbit::Packed<Edge<M>>> {
        self.raw.traverse()
    }

    pub(super) fn wait_for_scan(
        &mut self,
        counter: stat::Counter,
    ) -> Result<ribbit::Packed<Edge<M>>, ()> {
        self.raw.wait_for_scan(counter)
    }
}

impl<'g, 'l, R, M, V> Prefix<'g, 'l, R, M, V, path::Hybrid<'g, R, M>>
where
    R: key::Read<Edge = M>,
    M: edge::Meta,
    V: Value,
{
    #[cold]
    pub(super) fn freeze(&mut self) -> Option<ribbit::Packed<Edge<M>>> {
        self.raw.freeze()
    }
}
