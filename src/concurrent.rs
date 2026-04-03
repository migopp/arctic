mod iter;
mod key;
pub mod smr;
mod value;

use core::convert::Infallible;
use core::ops::ControlFlow;
use core::ops::RangeFull;
use core::sync::atomic::Ordering;

use smr::Global as _;
use smr::Guard as _;

use crate::raw;
use crate::raw::Cursor;
use crate::raw::Edge;
use crate::raw::Frozen;
use crate::raw::cursor;
use crate::raw::cursor::path;
use crate::raw::edge::Meta as _;
use crate::sequential;
use crate::stat;

pub use iter::EntryIter;
pub use iter::Prefix;
pub use iter::ValueIter;
pub use key::Key;
pub use smr::Smr;
pub use value::Value;

pub type Guard<'g, K, V, S> =
    <<S as Smr>::Global<<K as Key>::Prefix, V> as smr::Global<<K as Key>::Prefix, V>>::Guard<'g>;
pub type Owned<'g, K, V, S> = value::Owned<Guard<'g, K, V, S>, V>;
pub type Shared<'g, K, V, S> = value::Shared<Guard<'g, K, V, S>, V>;

pub struct Map<K: Key, V: Value, S: Smr = smr::Hazard> {
    smr: S::Global<K::Prefix, V>,
    inner: sequential::Map<K, V>,
}

unsafe impl<K: Key, V: Value + Send + Sync, S: Smr> Sync for Map<K, V, S> {}

impl<K: crate::Key, V: Value, S: Smr> Default for Map<K, V, S> {
    fn default() -> Self {
        Self {
            smr: S::Global::default(),
            inner: sequential::Map::<K, V>::default(),
        }
    }
}

impl<K: Key, V: Value, S: Smr> Map<K, V, S> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_smr(smr: S::Global<K::Prefix, V>) -> Self {
        Self {
            smr,
            inner: sequential::Map::<K, V>::default(),
        }
    }

    #[inline]
    pub fn as_sequential(&mut self) -> &mut sequential::Map<K, V> {
        &mut self.inner
    }

    #[inline]
    pub fn smr(&self) -> &S::Global<K::Prefix, V> {
        &self.smr
    }

    #[inline]
    pub fn smr_mut(&mut self) -> &mut S::Global<K::Prefix, V> {
        &mut self.smr
    }
}

pub enum Update<'g, K, V, B, S>
where
    K: Key,
    V: Value + 'g,
    S: Smr,
    S::Global<K::Prefix, V>: 'g,
{
    Absent {
        value: Option<V>,
    },
    Success {
        old: Owned<'g, K, V, S>,
    },
    Break {
        old: Shared<'g, K, V, S>,
        r#break: B,
    },
}

pub enum Remove<'g, K, V, S>
where
    K: Key,
    V: Value + 'g,
    S: Smr,
    S::Global<K::Prefix, V>: 'g,
{
    Absent,
    Success { old: Owned<'g, K, V, S> },
    Break,
}

pub enum Insert<'g, K, V, S, B = Option<V>>
where
    K: Key,
    V: Value + 'g,
    S: Smr,
    S::Global<K::Prefix, V>: 'g,
{
    Success {
        old: Option<Owned<'g, K, V, S>>,
    },
    Break {
        old: Option<Shared<'g, K, V, S>>,
        r#break: B,
    },
}

impl<K, V, S> Map<K, V, S>
where
    K: Key,
    V: Value + Send + Sync,
    S: Smr,
{
    #[inline]
    pub fn get(&self, key: K::Borrow<'_>) -> Option<Shared<'_, K, V, S>> {
        let reader = K::Read::from(key);
        let guard = self.smr.guard(K::hazard(reader));
        let value =
            unsafe { Cursor::<K, path::Discard>::new(self.inner.root(), reader).traverse_get()? };
        Some(unsafe { V::share(guard, value) })
    }

    #[inline]
    pub fn update(&self, key: K::Borrow<'_>, value: V) -> Result<Owned<'_, K, V, S>, V> {
        match self.update_with(key, Some(value), |_, new| {
            ControlFlow::<Infallible, _>::Continue(new)
        }) {
            Update::Absent {
                value: Some(initial),
            } => Err(initial),
            Update::Success { old } => Ok(old),
            Update::Absent { value: None } | Update::Break { .. } => unreachable!(),
        }
    }

    #[inline]
    pub fn update_with<F, B>(
        &self,
        key: K::Borrow<'_>,
        initial: Option<V>,
        mut update: F,
    ) -> Update<'_, K, V, B, S>
    where
        F: FnMut(V::Borrow<'_>, Option<V>) -> ControlFlow<B, Option<V>>,
    {
        let initial = if cfg!(feature = "opt-no-path") {
            initial
        } else {
            match self.update_with_optimistic(key, initial, &mut update) {
                Ok(update) => return update,
                Err(initial) => initial,
            }
        };

        self.update_with_pessimistic(key, initial, update)
    }

    #[inline]
    fn update_with_optimistic<F, B>(
        &self,
        key: K::Borrow<'_>,
        initial: Option<V>,
        update: F,
    ) -> Result<Update<'_, K, V, B, S>, Option<V>>
    where
        F: FnMut(V::Borrow<'_>, Option<V>) -> ControlFlow<B, Option<V>>,
    {
        self.update_with_impl::<path::Discard, _, _>(key, initial, update)
    }

    #[cold]
    fn update_with_pessimistic<F, B>(
        &self,
        key: K::Borrow<'_>,
        initial: Option<V>,
        update: F,
    ) -> Update<'_, K, V, B, S>
    where
        F: FnMut(V::Borrow<'_>, Option<V>) -> ControlFlow<B, Option<V>>,
    {
        stat::increment(stat::Counter::UpdatePessimistic);
        match self.update_with_impl::<path::Retain<_>, _, _>(key, initial, update) {
            Ok(update) => update,
            Err(_) => unreachable!(),
        }
    }

    #[inline]
    fn update_with_impl<'k, H, F, B>(
        &self,
        key: K::Borrow<'k>,
        mut initial: Option<V>,
        mut update: F,
    ) -> Result<Update<'_, K, V, B, S>, Option<V>>
    where
        H: path::History<'k, K>,
        F: FnMut(V::Borrow<'_>, Option<V>) -> ControlFlow<B, Option<V>>,
    {
        let reader = K::Read::from(key);
        let mut guard = self.smr.guard(K::hazard(reader));
        let mut cursor = unsafe { Cursor::<_, H>::new(self.inner.root(), reader) };

        loop {
            let old = match cursor.traverse_update() {
                None => return Ok(Update::Absent { value: initial }),
                Some(Ok(old)) => old,
                Some(Err(Frozen)) => match cursor.freeze() {
                    Err(_) => return Err(initial),
                    Ok(None) => continue,
                    Ok(Some(node)) => unsafe {
                        guard.retire_node(cursor.bits(), node);
                        continue;
                    },
                },
            };

            validate!(old.meta().is_value());

            let new = match update(
                unsafe { V::borrow_from_raw(old.into_value_unchecked()) },
                initial.take(),
            ) {
                ControlFlow::Continue(None) => Edge::DEFAULT,
                ControlFlow::Continue(Some(new)) => unsafe {
                    old.with_value_unchecked(V::into_raw(new))
                },
                ControlFlow::Break(r#break) => {
                    return Ok(Update::Break {
                        old: unsafe { V::share(guard, old.into_value_unchecked()) },
                        r#break,
                    });
                }
            };

            match cursor.edge().compare_exchange_packed(
                old,
                new,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(Update::Success {
                        old: unsafe { V::own(guard, old.into_value_unchecked()) },
                    });
                }
                Err(_) => {
                    initial = Some(unsafe { V::from_raw(new.into_value_unchecked()) });
                }
            }
        }
    }

    #[inline]
    pub fn remove_with<F>(&self, key: K::Borrow<'_>, with: F) -> Remove<'_, K, V, S>
    where
        F: FnMut(V::Borrow<'_>) -> bool,
    {
        let Ok(remove) = self.remove_with_impl::<true, path::Retain<'_, K>, _>(key, with);
        remove
    }

    #[inline]
    pub fn remove(&self, key: K::Borrow<'_>) -> Option<Owned<'_, K, V, S>> {
        match self.remove_with_impl::<true, path::Retain<'_, K>, _>(key, |_| true) {
            Ok(Remove::Absent) => None,
            Ok(Remove::Success { old }) => Some(old),
            Ok(Remove::Break) => unreachable!(),
        }
    }

    #[inline]
    fn remove_with_impl<'k, const RECURSE: bool, H, F>(
        &self,
        key: K::Borrow<'k>,
        mut remove: F,
    ) -> Result<Remove<'_, K, V, S>, H::PopError>
    where
        H: path::History<'k, K>,
        F: FnMut(V::Borrow<'_>) -> bool,
    {
        let reader = K::Read::from(key);
        let mut guard = self.smr.guard(K::hazard(reader));
        let mut cursor = unsafe { Cursor::<_, H>::new(self.inner.root(), reader) };

        let old = loop {
            let old = match cursor.traverse_update() {
                None => return Ok(Remove::Absent),
                Some(Ok(old)) => old,
                Some(Err(Frozen)) => match cursor.freeze()? {
                    None => continue,
                    Some(node) => unsafe {
                        guard.retire_node(cursor.bits(), node);
                        continue;
                    },
                },
            };

            validate!(old.meta().is_value());

            match remove(unsafe { V::borrow_from_raw(old.into_value_unchecked()) }) {
                true => (),
                false => return Ok(Remove::Break),
            }

            if cursor
                .edge()
                .compare_exchange_packed(old, Edge::DEFAULT, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                break unsafe { old.into_value_unchecked() };
            }
        };

        if RECURSE {
            cursor.reclaim()?;
        }

        Ok(Remove::Success {
            old: unsafe { V::own(guard, old) },
        })
    }

    #[inline]
    pub fn upsert(&self, key: K::Borrow<'_>, value: V) -> Option<Owned<'_, K, V, S>> {
        match self.insert_with(key, Some(value), |_, new| {
            ControlFlow::<Infallible, _>::Continue(new.expect("Value is always initialized"))
        }) {
            Insert::Success { old } => old,
            Insert::Break { .. } => unreachable!(),
        }
    }

    #[inline]
    pub fn insert(&self, key: K::Borrow<'_>, value: V) -> Result<(), (Shared<'_, K, V, S>, V)> {
        match self.insert_with(key, Some(value), |old, new| {
            let new = new.expect("Value is always initialized");
            match old {
                None => ControlFlow::Continue(new),
                Some(_) => ControlFlow::Break(new),
            }
        }) {
            Insert::Success { old } => {
                validate!(old.is_none());
                Ok(())
            }
            Insert::Break { old, r#break } => Err((old.expect("Break on `Some`"), r#break)),
        }
    }

    #[inline]
    pub fn insert_with<F, B>(
        &self,
        key: K::Borrow<'_>,
        initial: Option<V>,
        mut insert: F,
    ) -> Insert<'_, K, V, S, B>
    where
        F: FnMut(Option<V::Borrow<'_>>, Option<V>) -> ControlFlow<B, V>,
    {
        let initial = if cfg!(feature = "opt-no-path") {
            initial
        } else {
            match self.insert_with_optimistic(key, initial, &mut insert) {
                Ok(update) => return update,
                Err(initial) => initial,
            }
        };

        self.insert_with_pessimistic(key, initial, insert)
    }

    #[inline]
    fn insert_with_optimistic<F, B>(
        &self,
        key: K::Borrow<'_>,
        initial: Option<V>,
        insert: F,
    ) -> Result<Insert<'_, K, V, S, B>, Option<V>>
    where
        F: FnMut(Option<V::Borrow<'_>>, Option<V>) -> ControlFlow<B, V>,
    {
        self.insert_with_impl::<path::Discard, _, _>(key, initial, insert)
    }

    #[cold]
    fn insert_with_pessimistic<F, B>(
        &self,
        key: K::Borrow<'_>,
        initial: Option<V>,
        insert: F,
    ) -> Insert<'_, K, V, S, B>
    where
        F: FnMut(Option<V::Borrow<'_>>, Option<V>) -> ControlFlow<B, V>,
    {
        stat::increment(stat::Counter::InsertPessimistic);
        match self.insert_with_impl::<path::Retain<_>, _, _>(key, initial, insert) {
            Ok(upsert) => upsert,
            Err(_) => unreachable!(),
        }
    }

    #[inline]
    fn insert_with_impl<'k, H, F, B>(
        &self,
        key: K::Borrow<'k>,
        mut initial: Option<V>,
        mut insert: F,
    ) -> Result<Insert<'_, K, V, S, B>, Option<V>>
    where
        H: path::History<'k, K>,
        F: FnMut(Option<V::Borrow<'_>>, Option<V>) -> ControlFlow<B, V>,
    {
        let reader = K::Read::from(key);
        let mut guard = self.smr.guard(K::hazard(reader));
        let mut cursor = unsafe { Cursor::<_, H>::new(self.inner.root(), reader) };

        loop {
            match cursor.traverse_insert() {
                cursor::Insert::Value {
                    old_value,
                    old,
                    key,
                } => {
                    let new_value = match insert(
                        old_value.map(|old| unsafe { V::borrow_from_raw(old) }),
                        initial.take(),
                    ) {
                        ControlFlow::Continue(value) => V::into_raw(value),
                        ControlFlow::Break(r#break) => {
                            return Ok(Insert::Break {
                                old: old.as_value().map(|old| unsafe { V::share(guard, old) }),
                                r#break,
                            });
                        }
                    };

                    match cursor.insert(old, key, new_value) {
                        // Restore value and fall through to freeze
                        Err(Frozen) => initial = Some(unsafe { V::from_raw(new_value) }),

                        Ok(new) => match cursor.edge().compare_exchange_packed(
                            old,
                            new,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        ) {
                            Ok(_) => {
                                return Ok(Insert::Success {
                                    old: old_value.map(|old| unsafe { V::own(guard, old) }),
                                });
                            }
                            Err(_) => {
                                if let Some(node) = new.as_node() {
                                    unsafe {
                                        node.deallocate_recursive(stat::Counter::FreeConflict);
                                    }
                                }

                                initial = Some(unsafe { V::from_raw(new_value) });
                                continue;
                            }
                        },
                    }
                }
                cursor::Insert::Smo(Ok((smo, old, new))) => {
                    validate!(!old.meta().is_frozen());

                    match cursor.edge().compare_exchange_packed(
                        old,
                        new,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    ) {
                        Ok(_) => {
                            if let Some(node) = old.as_node() {
                                unsafe { guard.retire_node(cursor.bits(), node) };
                            }
                        }
                        Err(_) => {
                            // Does not go through SMR because `new` is still thread-local
                            if smo.is_allocate() {
                                let node = new.as_node().expect("Allocating SMO creates node");
                                unsafe {
                                    node.deallocate(stat::Counter::FreeConflict);
                                }
                            }
                        }
                    }

                    continue;
                }

                // Fall through to freeze
                cursor::Insert::Smo(Err(Frozen)) => (),
            }

            match cursor.freeze() {
                Err(_) => return Err(initial),
                Ok(None) => (),
                Ok(Some(node)) => unsafe { guard.retire_node(cursor.bits(), node) },
            }
        }
    }

    pub fn all(&self) -> iter::Prefix<'static, '_, K, V, RangeFull, Guard<'_, K, V, S>> {
        let guard = self.smr.guard(K::hazard(K::Read::default()));
        let prefix = unsafe { raw::iter::Prefix::<K>::new_all(self.inner.root()) };
        unsafe { Prefix::new(guard, prefix) }
    }

    pub fn prefix<'k>(
        &self,
        prefix: impl Into<K::Read<'k>>,
    ) -> Option<iter::Prefix<'k, '_, K, V, RangeFull, Guard<'_, K, V, S>>> {
        let prefix = prefix.into();
        let guard = self.smr.guard(K::hazard(prefix));
        let prefix = unsafe { raw::iter::Prefix::<K>::new_prefix(self.inner.root(), prefix) }?;
        Some(unsafe { Prefix::new(guard, prefix) })
    }

    pub fn range<'k, R>(
        &self,
        range: R,
    ) -> Option<iter::Prefix<'k, '_, K, V, R, Guard<'_, K, V, S>>>
    where
        R: crate::raw::iter::Range<'k, K>,
    {
        // FIXME: avoid recomputing common prefix?
        let guard = self.smr.guard(K::hazard(range.common_prefix()));
        let prefix = unsafe { raw::iter::Prefix::new_range(self.inner.root(), range) }?;
        Some(unsafe { Prefix::new(guard, prefix) })
    }
}

impl<K, V, S> From<sequential::Map<K, V>> for Map<K, V, S>
where
    K: Key,
    V: Value,
    S: Smr,
{
    fn from(inner: sequential::Map<K, V>) -> Self {
        Self {
            smr: S::Global::default(),
            inner,
        }
    }
}

impl<K, V, S> From<Map<K, V, S>> for sequential::Map<K, V>
where
    K: Key,
    V: Value,
    S: Smr,
{
    fn from(map: Map<K, V, S>) -> sequential::Map<K, V> {
        map.inner
    }
}
