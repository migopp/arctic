use core::fmt;
use core::marker::PhantomData;
#[cfg_attr(feature = "smr-epoch", expect(unused_imports))]
use core::sync::atomic::Ordering;

use crate::concurrent;
use crate::concurrent::hazard;
use crate::concurrent::hazard::Prefix as _;
use crate::raw::edge;
use crate::raw::Edge;

pub struct Traverse<'g, 'l, P: ribbit::Pack<Packed: hazard::Prefix>, V: concurrent::Value> {
    _local: PhantomData<&'l mut &'g V>,

    #[cfg(feature = "smr-epoch")]
    guard: crossbeam_epoch::Guard,

    #[cfg(not(feature = "smr-epoch"))]
    local: &'l mut hazard::Local<'g, P, V>,
}

impl<P: ribbit::Pack<Packed: hazard::Prefix>, V: concurrent::Value> Drop
    for Traverse<'_, '_, P, V>
{
    #[inline]
    fn drop(&mut self) {
        if cfg!(feature = "smr-disable") {
            return;
        }

        #[cfg(not(feature = "smr-epoch"))]
        self.local.hazard.store_packed(
            ribbit::Packed::<P>::HAZARD_NULL,
            core::sync::atomic::Ordering::Relaxed,
        );
    }
}

impl<'g, 'l, P: ribbit::Pack<Packed: hazard::Prefix>, V: concurrent::Value> Traverse<'g, 'l, P, V> {
    pub(super) fn new(local: &'l mut hazard::Local<'g, P, V>) -> Self {
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
    pub(crate) unsafe fn guard_owned(self, value: V::Borrow<'l>) -> Value<'g, 'l, true, P, V> {
        if cfg!(feature = "smr-disable") {
            return Value {
                inner: self,
                borrow: value,
            };
        }

        #[cfg(not(feature = "smr-epoch"))]
        {
            let hazard = self.local.hazard.load_packed(Ordering::Relaxed);
            self.local
                .hazard
                .store_packed(hazard.without_overlap().without_node(), Ordering::Relaxed);
        }

        Value {
            inner: self,
            borrow: value,
        }
    }

    #[inline]
    pub(crate) fn guard_shared(self, value: V::Borrow<'l>) -> Value<'g, 'l, false, P, V> {
        if cfg!(feature = "smr-disable") {
            return Value {
                inner: self,
                borrow: value,
            };
        }

        #[cfg(not(feature = "smr-epoch"))]
        {
            let hazard = self.local.hazard.load_packed(Ordering::Relaxed);
            self.local
                .hazard
                .store_packed(hazard.without_overlap().without_node(), Ordering::Relaxed);
        }

        Value {
            inner: self,
            borrow: value,
        }
    }

    #[inline]
    pub(crate) fn guard_prefix(self) -> Prefix<'g, 'l, P, V> {
        if cfg!(feature = "smr-disable") {
            return Prefix(self);
        }

        #[cfg(not(feature = "smr-epoch"))]
        {
            let hazard = self.local.hazard.load_packed(Ordering::Relaxed);
            self.local
                .hazard
                .store_packed(hazard.without_overlap(), Ordering::Relaxed);
        }

        Prefix(self)
    }

    #[expect(unused)]
    #[inline]
    pub(crate) fn guard_linearizable(self) -> Values<'g, 'l, P, V> {
        if cfg!(feature = "smr-disable") {
            return Values(self);
        }

        #[cfg(not(feature = "smr-epoch"))]
        {
            let hazard = self.local.hazard.load_packed(Ordering::Relaxed);
            self.local
                .hazard
                .store_packed(hazard.without_node(), Ordering::Relaxed);
        }

        Values(self)
    }
}

pub struct Prefix<'g, 'l, P: ribbit::Pack<Packed: hazard::Prefix>, V: concurrent::Value>(
    Traverse<'g, 'l, P, V>,
);

impl<'g, 'l, P: ribbit::Pack<Packed: hazard::Prefix>, V: concurrent::Value> core::ops::Deref
    for Prefix<'g, 'l, P, V>
{
    type Target = Traverse<'g, 'l, P, V>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub struct Values<'g, 'l, P: ribbit::Pack<Packed: hazard::Prefix>, V: concurrent::Value>(
    Traverse<'g, 'l, P, V>,
);

impl<'g, 'l, P: ribbit::Pack<Packed: hazard::Prefix>, V: concurrent::Value> core::ops::Deref
    for Values<'g, 'l, P, V>
{
    type Target = Traverse<'g, 'l, P, V>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub struct Value<
    'g,
    'l,
    const OWNED: bool,
    P: ribbit::Pack<Packed: hazard::Prefix>,
    V: concurrent::Value,
> {
    inner: Traverse<'g, 'l, P, V>,
    borrow: V::Borrow<'l>,
}

impl<'l, const OWNED: bool, P: ribbit::Pack<Packed: hazard::Prefix>, V> fmt::Debug
    for Value<'_, 'l, OWNED, P, V>
where
    V: concurrent::Value,
    V::Borrow<'l>: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.borrow.fmt(f)
    }
}

impl<'g, 'l, const OWNED: bool, P: ribbit::Pack<Packed: hazard::Prefix>, V> core::ops::Deref
    for Value<'g, 'l, OWNED, P, V>
where
    V: concurrent::Value,
{
    type Target = V::Borrow<'l>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.borrow
    }
}

impl<'g, 'l, const OWNED: bool, P: ribbit::Pack<Packed: hazard::Prefix>, V> Drop
    for Value<'g, 'l, OWNED, P, V>
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
