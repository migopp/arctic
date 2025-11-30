use core::fmt;
use core::marker::PhantomData;
#[cfg_attr(feature = "smr-epoch", expect(unused_imports))]
use core::sync::atomic::Ordering;

use crate::concurrent;
use crate::concurrent::hazard;
use crate::concurrent::hazard::Prefix as _;
use crate::raw::edge;
use crate::raw::Edge;

pub struct Traverse<'g, 'l, V: concurrent::Value> {
    _local: PhantomData<&'l mut &'g V>,

    #[cfg(feature = "smr-epoch")]
    guard: crossbeam_epoch::Guard,

    #[cfg(not(feature = "smr-epoch"))]
    local: &'l mut hazard::Local<'g, V>,
}

impl<V: concurrent::Value> Drop for Traverse<'_, '_, V> {
    #[inline]
    fn drop(&mut self) {
        if cfg!(feature = "smr-disable") {
            return;
        }

        #[cfg(not(feature = "smr-epoch"))]
        self.local.hazard.store_packed(
            ribbit::Packed::<hazard::prefix::Be>::HAZARD_NULL,
            core::sync::atomic::Ordering::Relaxed,
        );
    }
}

impl<'g, 'l, V: concurrent::Value> Traverse<'g, 'l, V> {
    pub(super) fn new(local: &'l mut hazard::Local<'g, V>) -> Self {
        Self {
            _local: PhantomData,

            #[cfg(feature = "smr-epoch")]
            guard: local.handle.pin(),

            #[cfg(not(feature = "smr-epoch"))]
            local,
        }
    }

    pub(crate) unsafe fn retire<M: ribbit::Pack<Packed: edge::Meta>>(
        &mut self,
        _bits: usize,
        edge: ribbit::Packed<Edge<M>>,
    ) {
        if cfg!(feature = "smr-disable") {
            return;
        }

        #[cfg(feature = "smr-epoch")]
        unsafe {
            self.guard
                .defer_unchecked(move || hazard::deallocate_epoch::<M, V>(edge));
        }

        #[cfg(not(feature = "smr-epoch"))]
        self.local.retire_edge(_bits, edge);
    }

    /// # SAFETY
    ///
    /// Caller must ensure that only one thread calls this for any given value.
    #[inline]
    pub(crate) unsafe fn guard_owned(self, value: V::Borrow<'l>) -> Value<'g, 'l, true, V> {
        if cfg!(feature = "smr-disable") {
            return Value {
                inner: self,
                borrow: value,
            };
        }

        #[cfg(not(feature = "smr-epoch"))]
        {
            let hazard = self.local.hazard.load_packed(Ordering::Relaxed);
            self.local.hazard.store_packed(
                hazard.with_overlap(false).with_node(false).with_value(true),
                Ordering::Relaxed,
            );
        }

        Value {
            inner: self,
            borrow: value,
        }
    }

    #[inline]
    pub(crate) fn guard_shared(self, value: V::Borrow<'l>) -> Value<'g, 'l, false, V> {
        if cfg!(feature = "smr-disable") {
            return Value {
                inner: self,
                borrow: value,
            };
        }

        #[cfg(not(feature = "smr-epoch"))]
        {
            let hazard = self.local.hazard.load_packed(Ordering::Relaxed);
            self.local.hazard.store_packed(
                hazard.with_overlap(false).with_node(false).with_value(true),
                Ordering::Relaxed,
            );
        }

        Value {
            inner: self,
            borrow: value,
        }
    }

    #[inline]
    pub(crate) fn guard_prefix(self) -> Prefix<'g, 'l, V> {
        if cfg!(feature = "smr-disable") {
            return Prefix(self);
        }

        #[cfg(not(feature = "smr-epoch"))]
        {
            let hazard = self.local.hazard.load_packed(Ordering::Relaxed);
            self.local
                .hazard
                .store_packed(hazard.with_overlap(false), Ordering::Relaxed);
        }

        Prefix(self)
    }

    #[inline]
    pub(crate) fn guard_linearizable(self) -> Values<'g, 'l, V> {
        if cfg!(feature = "smr-disable") {
            return Values(self);
        }

        #[cfg(not(feature = "smr-epoch"))]
        {
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
    borrow: V::Borrow<'l>,
}

impl<'l, const OWNED: bool, V> fmt::Debug for Value<'_, 'l, OWNED, V>
where
    V: concurrent::Value,
    V::Borrow<'l>: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.borrow.fmt(f)
    }
}

impl<'g, 'l, const OWNED: bool, V> core::ops::Deref for Value<'g, 'l, OWNED, V>
where
    V: concurrent::Value,
{
    type Target = V::Borrow<'l>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.borrow
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

        if !OWNED {
            return;
        }

        unsafe {
            let raw = V::borrow_into_raw(self.borrow);

            #[cfg(feature = "smr-epoch")]
            self.inner.guard.defer_unchecked(move || V::from_raw(raw));

            #[cfg(not(feature = "smr-epoch"))]
            // NOTE: could technically unguard before retiring, since
            // we will not access `value` anymore, but then we'd want
            // to avoid dropping `self.inner`.
            self.inner.local.retire_value(raw)
        }
    }
}
