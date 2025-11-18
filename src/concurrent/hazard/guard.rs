use core::fmt;
use core::sync::atomic::Ordering;

use crate::concurrent;
use crate::concurrent::hazard;
use crate::concurrent::hazard::prefix;
use crate::raw::edge;
use crate::raw::Edge;

pub struct Traverse<'g, 'l, V: concurrent::Value> {
    local: &'l mut hazard::Local<'g, V>,
}

impl<V: concurrent::Value> Drop for Traverse<'_, '_, V> {
    #[inline]
    fn drop(&mut self) {
        if cfg!(feature = "smr-disable") {
            return;
        }

        self.local
            .hazard
            .store_packed(prefix::Be::HAZARD_NULL, Ordering::Relaxed);
    }
}

impl<'g, 'l, V: concurrent::Value> Traverse<'g, 'l, V> {
    pub(super) fn new(local: &'l mut hazard::Local<'g, V>) -> Self {
        Self { local }
    }

    pub(crate) unsafe fn retire<M: ribbit::Pack<Packed: edge::Meta>>(
        &mut self,
        bits: usize,
        edge: ribbit::Packed<Edge<M>>,
    ) {
        if cfg!(feature = "smr-disable") {
            return;
        }

        self.local.retire_edge(bits, edge);
    }

    /// # SAFETY
    ///
    /// Caller must ensure that only one thread calls this for any given value.
    #[inline]
    pub(crate) unsafe fn guard_owned(self, value: V::Borrow<'l>) -> Value<'g, 'l, true, V> {
        if !cfg!(feature = "smr-disable") {
            let hazard = self.local.hazard.load_packed(Ordering::Relaxed);
            self.local.hazard.store_packed(
                hazard.with_overlap(false).with_node(false).with_value(true),
                Ordering::Relaxed,
            );
        }

        Value { inner: self, value }
    }

    #[inline]
    pub(crate) fn guard_shared(self, value: V::Borrow<'l>) -> Value<'g, 'l, false, V> {
        if !cfg!(feature = "smr-disable") {
            let hazard = self.local.hazard.load_packed(Ordering::Relaxed);
            self.local.hazard.store_packed(
                hazard.with_overlap(false).with_node(false).with_value(true),
                Ordering::Relaxed,
            );
        }

        Value { inner: self, value }
    }

    #[inline]
    pub(crate) fn guard_prefix(self) -> Prefix<'g, 'l, V> {
        if !cfg!(feature = "smr-disable") {
            let hazard = self.local.hazard.load_packed(Ordering::Relaxed);
            self.local
                .hazard
                .store_packed(hazard.with_overlap(false), Ordering::Relaxed);
        }

        Prefix(self)
    }

    #[inline]
    pub(crate) fn guard_linearizable(self) -> Values<'g, 'l, V> {
        if !cfg!(feature = "smr-disable") {
            let hazard = self.local.hazard.load_packed(Ordering::Relaxed);
            self.local
                .hazard
                .store_packed(hazard.with_node(false), Ordering::Relaxed);
        }

        Values(self)
    }
}

pub struct Prefix<'g, 'l, V: concurrent::Value>(Traverse<'g, 'l, V>);

impl<'g, 'l, V: concurrent::Value> core::ops::Deref for Prefix<'g, 'l, V> {
    type Target = Traverse<'g, 'l, V>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub struct Values<'g, 'l, V: concurrent::Value>(Traverse<'g, 'l, V>);

impl<'g, 'l, V: concurrent::Value> core::ops::Deref for Values<'g, 'l, V> {
    type Target = Traverse<'g, 'l, V>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub struct Value<'g, 'l, const OWNED: bool, V: concurrent::Value> {
    inner: Traverse<'g, 'l, V>,
    value: V::Borrow<'l>,
}

impl<'l, const OWNED: bool, V> fmt::Debug for Value<'_, 'l, OWNED, V>
where
    V: concurrent::Value,
    V::Borrow<'l>: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)
    }
}

impl<'g, 'l, const OWNED: bool, V> core::ops::Deref for Value<'g, 'l, OWNED, V>
where
    V: concurrent::Value,
{
    type Target = V::Borrow<'l>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<'g, 'l, const OWNED: bool, V> Drop for Value<'g, 'l, OWNED, V>
where
    V: concurrent::Value,
{
    fn drop(&mut self) {
        if cfg!(feature = "smr-disable") {
            return;
        }

        if OWNED {
            // NOTE: could technically unguard before retiring, since
            // we will not access `value` anymore, but then we'd want
            // to avoid dropping `self.inner`.
            unsafe {
                self.inner
                    .local
                    .retire_value(V::borrow_into_raw(self.value))
            }
        }
    }
}
