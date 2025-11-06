use ribbit::atomic::Atomic128;

use crate::byte;
use crate::concurrent::smr;
use crate::key;
use crate::raw;
pub(super) use crate::raw::cursor::path;
use crate::raw::Edge;
use crate::raw::Op;
use crate::stat;
use crate::value;
use crate::Value;

/// Tree traversal state.
pub(crate) struct Point<'g, 'l, R, C, V: Value, H> {
    /// SMR guard protecting allocations that overlap with `key`
    guard: smr::TraverseGuard<'g, 'l, V>,

    raw: crate::raw::cursor::Point<'g, R, C, H>,
}

impl<'g, 'l, R, C, V, H> Point<'g, 'l, R, C, V, H>
where
    R: key::Read,
    V: Value,
    H: path::History<'g, R, C>,
{
    #[inline]
    pub(crate) fn new(
        smr: &'l mut smr::Local<'g, V>,
        root: &'g Atomic128<Edge<C>>,
        key: R,
    ) -> Self {
        Self {
            guard: smr.guard(key.peek_all()),
            raw: unsafe { raw::cursor::Point::new(root, key) },
        }
    }

    #[inline]
    pub(crate) fn edge(&self) -> &'g Atomic128<Edge<C>> {
        self.raw.edge()
    }

    #[inline]
    pub(crate) unsafe fn retire(&mut self, edge: ribbit::Packed<Edge<C>>) {
        let prefix = self.guard.prefix();
        let key = prefix.truncate(byte::Len::MAX.min_bits(self.raw.bits()));
        unsafe {
            self.guard
                .retire(edge.with_meta(edge.meta().with_key(key)).erase())
        }
    }

    #[inline]
    pub(crate) fn into_guard(self) -> smr::TraverseGuard<'g, 'l, V> {
        self.guard
    }

    #[inline]
    pub(crate) fn traverse_exact(&mut self) -> Option<Result<ribbit::Packed<Edge<C>>, ()>> {
        self.raw.traverse_exact()
    }

    #[inline]
    pub(crate) fn traverse_or_insert(
        &mut self,
        value: value::Raw<V>,
    ) -> Result<(Op, ribbit::Packed<Edge<C>>, ribbit::Packed<Edge<C>>), ()> {
        self.raw.traverse_or_insert(u64::from(value))
    }

    #[cold]
    pub(crate) fn freeze(&mut self) -> Result<(), H::PopError> {
        if let Some(edge) = self.raw.freeze()? {
            unsafe {
                self.retire(edge);
            }
        }

        Ok(())
    }

    #[cold]
    pub(crate) fn wait_for_scan(
        &self,
        counter: stat::Counter,
    ) -> Result<ribbit::Packed<Edge<C>>, ()> {
        self.raw.wait_for_scan(counter)
    }
}

impl<'g, 'l, R, C, V> Point<'g, 'l, R, C, V, path::Discard>
where
    R: key::Read,
    V: Value,
{
    #[inline]
    pub(crate) fn get(
        smr: &'l mut smr::Local<'g, V>,
        root: &'g Atomic128<Edge<C>>,
        key: R,
    ) -> Option<V::SharedGuard<'g, 'l>>
    where
        R: key::Read,
        V: Value,
    {
        let guard = smr.guard(key.peek_all());
        let value = unsafe { crate::raw::cursor::Point::get(root, key)? };
        return Some(unsafe { V::guard_shared(guard, value) });
    }
}

pub(crate) struct Prefix<'g, 'l, R, C, V: Value, H> {
    /// SMR guard protecting allocations that overlap with `key`
    guard: smr::TraverseGuard<'g, 'l, V>,

    raw: crate::raw::cursor::Prefix<'g, R, C, H>,
}

impl<'g, 'l, R, C, V, H> Prefix<'g, 'l, R, C, V, H>
where
    R: key::Read,
    V: Value,
    H: path::History<'g, R, C>,
{
    pub(crate) fn new_root(smr: &'l mut smr::Local<'g, V>, root: &'g Atomic128<Edge<C>>) -> Self {
        Self {
            guard: smr.guard(R::default().peek_all()),
            raw: unsafe { crate::raw::cursor::Prefix::new_root(root) },
        }
    }

    pub(crate) fn new_prefix(
        smr: &'l mut smr::Local<'g, V>,
        root: &'g Atomic128<Edge<C>>,
        prefix: R,
    ) -> Option<Self> {
        let guard = smr.guard(prefix.peek_all());
        Some(Self {
            guard,
            raw: unsafe { crate::raw::cursor::Prefix::new_prefix(root, prefix) }?,
        })
    }

    pub(crate) fn new_range(
        smr: &'l mut smr::Local<'g, V>,
        root: &'g Atomic128<Edge<C>>,
        min: R,
        max: R,
    ) -> Option<Self> {
        let prefix = min.prefix(&max);
        Self::new_prefix(smr, root, prefix)
    }

    pub(crate) fn prefix(&self) -> R {
        self.raw.prefix()
    }

    pub(crate) fn edge(&self) -> &'g Atomic128<Edge<C>> {
        self.raw.edge()
    }

    #[inline]
    pub(crate) fn into_guard(self) -> smr::TraverseGuard<'g, 'l, V> {
        self.guard
    }

    pub(crate) fn traverse(&mut self) -> Option<ribbit::Packed<Edge<C>>> {
        self.raw.traverse()
    }

    pub(crate) fn wait_for_scan(
        &mut self,
        counter: stat::Counter,
    ) -> Result<ribbit::Packed<Edge<C>>, ()> {
        self.raw.wait_for_scan(counter)
    }
}

impl<'g, 'l, R: key::Read, C, V: Value> Prefix<'g, 'l, R, C, V, path::Hybrid<'g, R, C>> {
    #[cold]
    pub(crate) fn freeze(&mut self) -> Option<ribbit::Packed<Edge<C>>> {
        self.raw.freeze()
    }
}
