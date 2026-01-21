use ribbit::Atomic;

use crate::concurrent::smr;
use crate::raw;
use crate::raw::cursor::path;
use crate::raw::node;
use crate::raw::Edge;
use crate::raw::Frozen;
use crate::raw::Smo;
use crate::Key;

/// Tree traversal state.
pub(super) struct Cursor<'k, 'g, 'l, K: Key, H, G> {
    /// SMR guard protecting allocations that overlap with `key`
    guard: &'l mut G,

    raw: raw::Cursor<'k, 'g, K, H>,
}

impl<'k, 'g, 'l, K, H, G> Cursor<'k, 'g, 'l, K, H, G>
where
    K: Key,
    H: path::History<'k, 'g, K>,
    G: smr::Guard,
{
    #[inline]
    pub(super) fn new(
        guard: &'l mut G,
        root: &'g Atomic<Edge<K::Edge>>,
        key: K::Read<'k>,
    ) -> Cursor<'k, 'g, 'l, K, H, G> {
        Cursor {
            guard,
            raw: unsafe { raw::Cursor::new(root, key) },
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
    pub(super) unsafe fn retire_node(&mut self, node: ribbit::Packed<node::Ptr<K::Edge>>) {
        unsafe { self.guard.retire_node(self.raw.bits(), node) }
    }

    // #[inline]
    // pub(super) fn into_guard(self) -> hazard::guard::Traverse<'g, 'l, K::Prefix, V> {
    //     self.guard
    // }

    #[inline]
    pub(super) fn traverse_get(self) -> Option<u64> {
        unsafe { self.raw.traverse_get() }
    }

    #[inline]
    pub(super) fn traverse_update(
        &mut self,
    ) -> Option<Result<ribbit::Packed<Edge<K::Edge>>, Frozen>> {
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
        Frozen,
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
                self.retire_node(edge);
            }
        }

        Ok(())
    }
}
