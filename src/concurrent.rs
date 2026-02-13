mod iter;
mod key;
pub mod smr;
mod value;

use core::convert::Infallible;
use core::ops::ControlFlow;
use core::ops::RangeFull;
use core::sync::atomic::Ordering;

use polonius_the_crab::exit_polonius;
use polonius_the_crab::polonius;
use polonius_the_crab::polonius_return;
use smr::Guard as _;
use smr::Local as _;

use crate::raw::cursor;
use crate::raw::cursor::path;
use crate::raw::edge::Meta as _;
use crate::raw::key::Read as _;
use crate::raw::Cursor;
use crate::raw::Edge;
use crate::raw::Frozen;
use crate::sequential;
use crate::stat;

pub use iter::EntryIter;
pub use iter::Prefix;
pub use iter::ValueIter;
pub use key::Key;
pub use smr::Smr;
pub use value::Value;

pub type Guard<'g, 'l, K, V, S> =
    <<S as Smr<<K as Key>::Prefix, V>>::Local<'g> as smr::Local<<K as Key>::Prefix, V>>::Guard<'l>;
pub type Owned<'g, 'l, K, V, S> = value::Owned<'l, Guard<'g, 'l, K, V, S>, V>;
pub type Shared<'g, 'l, K, V, S> = value::Shared<'l, Guard<'g, 'l, K, V, S>, V>;

pub struct Map<K: Key, V: Value, S = smr::Hazard<<K as Key>::Prefix, V>> {
    smr: S,
    raw: sequential::Map<K, V>,
}

unsafe impl<K: Key, V: Value + Send + Sync, S: Smr<K::Prefix, V>> Sync for Map<K, V, S> {}

impl<K: crate::Key, V: Value, S: Smr<K::Prefix, V>> Default for Map<K, V, S> {
    fn default() -> Self {
        Self {
            smr: S::default(),
            raw: sequential::Map::<K, V>::default(),
        }
    }
}

impl<K: Key, V: Value, S: Smr<K::Prefix, V>> Map<K, V, S> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_smr(smr: S) -> Self {
        Self {
            smr,
            raw: sequential::Map::<K, V>::default(),
        }
    }

    #[inline]
    pub fn pin(&self) -> MapRef<'_, K, V, S> {
        MapRef {
            smr: self.smr.local(),
            raw: &self.raw,
        }
    }

    #[inline]
    pub fn as_sequential(&mut self) -> &mut sequential::Map<K, V> {
        &mut self.raw
    }

    #[inline]
    pub fn smr(&mut self) -> &mut S {
        &mut self.smr
    }
}

pub struct MapRef<
    'g,
    K: Key,
    V: Value,
    S: 'g + Smr<K::Prefix, V> = smr::Hazard<<K as Key>::Prefix, V>,
> {
    smr: S::Local<'g>,
    raw: &'g sequential::Map<K, V>,
}

pub enum Update<'g, 'l, K, V, B, S = smr::Hazard<<K as Key>::Prefix, V>>
where
    K: Key,
    V: Value,
    S: Smr<K::Prefix, V> + 'g,
    S::Local<'g>: 'l,
{
    Absent {
        value: Option<V>,
    },
    Success {
        old: Owned<'g, 'l, K, V, S>,
    },
    Break {
        old: Shared<'g, 'l, K, V, S>,
        r#break: B,
    },
}

pub enum Insert<'g, 'l, K, V, B = Option<V>, S = smr::Hazard<<K as Key>::Prefix, V>>
where
    K: Key,
    V: Value,
    S: Smr<K::Prefix, V> + 'g,
    S::Local<'g>: 'l,
{
    Success {
        old: Option<Owned<'g, 'l, K, V, S>>,
    },
    Break {
        old: Option<Shared<'g, 'l, K, V, S>>,
        r#break: B,
    },
}

impl<'g, K, V, S> MapRef<'g, K, V, S>
where
    K: Key,
    V: Value + Send + Sync,
    S: Smr<K::Prefix, V>,
{
    #[inline]
    pub fn smr(&self) -> &S::Local<'g> {
        &self.smr
    }

    #[inline]
    pub fn get(&mut self, key: K::Borrow<'_>) -> Option<Shared<'g, '_, K, V, S>> {
        let reader = K::Read::from(key);
        let guard = self.smr.guard(K::hazard(reader));
        let value =
            unsafe { Cursor::<K, path::Discard>::new(self.raw.root(), reader).traverse_get()? };
        Some(unsafe { V::share(guard, value) })
    }

    #[inline]
    pub fn remove(&mut self, key: K::Borrow<'_>) -> Option<Owned<'g, '_, K, V, S>> {
        match self.update_with(key, None, |_, _| {
            ControlFlow::<Infallible, _>::Continue(None)
        }) {
            Update::Absent { .. } => None,
            Update::Success { old } => Some(old),
            Update::Break { .. } => unreachable!(),
        }
    }

    #[inline]
    pub fn remove_with<F>(&mut self, key: K::Borrow<'_>, mut with: F) -> Update<'g, '_, K, V, (), S>
    where
        F: FnMut(V::Borrow<'_>) -> bool,
    {
        self.update_with(key, None, |old, _| {
            if with(old) {
                ControlFlow::Continue(None)
            } else {
                ControlFlow::Break(())
            }
        })
    }

    #[inline]
    pub fn update(&mut self, key: K::Borrow<'_>, value: V) -> Result<Owned<'g, '_, K, V, S>, V> {
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
        &mut self,
        key: K::Borrow<'_>,
        initial: Option<V>,
        mut update: F,
    ) -> Update<'g, '_, K, V, B, S>
    where
        F: FnMut(V::Borrow<'_>, Option<V>) -> ControlFlow<B, Option<V>>,
    {
        let mut map = self;

        let initial = if cfg!(feature = "opt-no-path") {
            initial
        } else {
            // Cursed workaround for:
            // https://github.com/rust-lang/rust/issues/54663
            polonius!(|map| -> Update<'g, 'polonius, K, V, B, S> {
                match map.update_with_optimistic(key, initial, &mut update) {
                    Ok(update) => polonius_return!(update),
                    Err(initial) => exit_polonius!(initial),
                }
            })
        };

        map.update_with_pessimistic(key, initial, update)
    }

    #[inline]
    fn update_with_optimistic<F, B>(
        &mut self,
        key: K::Borrow<'_>,
        initial: Option<V>,
        update: F,
    ) -> Result<Update<'g, '_, K, V, B, S>, Option<V>>
    where
        F: FnMut(V::Borrow<'_>, Option<V>) -> ControlFlow<B, Option<V>>,
    {
        self.update_with_impl::<path::Discard, _, _>(key, initial, update)
    }

    #[cold]
    fn update_with_pessimistic<F, B>(
        &mut self,
        key: K::Borrow<'_>,
        initial: Option<V>,
        update: F,
    ) -> Update<'g, '_, K, V, B, S>
    where
        F: FnMut(V::Borrow<'_>, Option<V>) -> ControlFlow<B, Option<V>>,
    {
        match self.update_with_impl::<path::Retain<_>, _, _>(key, initial, update) {
            Ok(update) => update,
            Err(_) => unreachable!(),
        }
    }

    #[inline]
    fn update_with_impl<'k, H, F, B>(
        &mut self,
        key: K::Borrow<'k>,
        mut initial: Option<V>,
        mut update: F,
    ) -> Result<Update<'g, '_, K, V, B, S>, Option<V>>
    where
        H: path::History<'k, 'g, K>,
        F: FnMut(V::Borrow<'_>, Option<V>) -> ControlFlow<B, Option<V>>,
    {
        let reader = K::Read::from(key);
        let mut guard = self.smr.guard(K::hazard(reader));
        let mut cursor = unsafe { Cursor::<_, H>::new(self.raw.root(), reader) };

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
                unsafe { V::borrow_from_raw(old.into_raw()) },
                initial.take(),
            ) {
                ControlFlow::Continue(None) => Edge::DEFAULT,
                ControlFlow::Continue(Some(new)) => old.with_value(V::into_raw(new)),
                ControlFlow::Break(r#break) => {
                    return Ok(Update::Break {
                        old: unsafe { V::share(guard, old.into_raw()) },
                        r#break,
                    })
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
                        old: unsafe { V::own(guard, old.into_raw()) },
                    })
                }
                Err(_) => {
                    initial = Some(unsafe { V::from_raw(new.into_raw()) });
                }
            }
        }
    }

    #[inline]
    pub fn upsert(&mut self, key: K::Borrow<'_>, value: V) -> Option<Owned<'g, '_, K, V, S>> {
        match self.insert_with(key, Some(value), |_, new| {
            ControlFlow::<Infallible, _>::Continue(new.expect("Value is always initialized"))
        }) {
            Insert::Success { old } => old,
            Insert::Break { .. } => unreachable!(),
        }
    }

    #[inline]
    pub fn insert(
        &mut self,
        key: K::Borrow<'_>,
        value: V,
    ) -> Result<(), (Shared<'g, '_, K, V, S>, V)> {
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
        &mut self,
        key: K::Borrow<'_>,
        initial: Option<V>,
        mut insert: F,
    ) -> Insert<'g, '_, K, V, B, S>
    where
        F: FnMut(Option<V::Borrow<'_>>, Option<V>) -> ControlFlow<B, V>,
    {
        let mut map = &mut *self;

        let initial = if cfg!(feature = "opt-no-path") {
            initial
        } else {
            // Cursed workaround for:
            // https://github.com/rust-lang/rust/issues/54663
            polonius!(|map| -> Insert<'g, 'polonius, K, V, B, S> {
                match map.insert_with_optimistic(key, initial, &mut insert) {
                    Ok(update) => polonius_return!(update),
                    Err(initial) => exit_polonius!(initial),
                }
            })
        };

        map.insert_with_pessimistic(key, initial, insert)
    }

    #[inline]
    fn insert_with_optimistic<F, B>(
        &mut self,
        key: K::Borrow<'_>,
        initial: Option<V>,
        insert: F,
    ) -> Result<Insert<'g, '_, K, V, B, S>, Option<V>>
    where
        F: FnMut(Option<V::Borrow<'_>>, Option<V>) -> ControlFlow<B, V>,
    {
        self.insert_with_impl::<path::Discard, _, _>(key, initial, insert)
    }

    #[cold]
    fn insert_with_pessimistic<F, B>(
        &mut self,
        key: K::Borrow<'_>,
        initial: Option<V>,
        insert: F,
    ) -> Insert<'g, '_, K, V, B, S>
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
        &mut self,
        key: K::Borrow<'k>,
        mut initial: Option<V>,
        mut insert: F,
    ) -> Result<Insert<'g, '_, K, V, B, S>, Option<V>>
    where
        H: path::History<'k, 'g, K>,
        F: FnMut(Option<V::Borrow<'_>>, Option<V>) -> ControlFlow<B, V>,
    {
        let reader = K::Read::from(key);
        let mut guard = self.smr.guard(K::hazard(reader));
        let mut cursor = unsafe { Cursor::<_, H>::new(self.raw.root(), reader) };

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
                            })
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
                            stat::increment(smo);
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

    pub fn all(&mut self) -> iter::Prefix<'static, 'g, K, V, RangeFull, Guard<'g, '_, K, V, S>> {
        let guard = self.smr.guard(K::hazard(K::Read::default()));
        unsafe { iter::Prefix::new(guard, self.raw.root(), K::Read::default(), ..) }
    }

    pub fn prefix<'k>(
        &mut self,
        prefix: impl Into<K::Read<'k>>,
    ) -> Option<iter::Prefix<'k, 'g, K, V, RangeFull, Guard<'g, '_, K, V, S>>> {
        let prefix = prefix.into();
        let guard = self.smr.guard(K::hazard(prefix));
        let mut cursor = unsafe { Cursor::<K, path::Discard>::new(self.raw.root(), prefix) };
        cursor.traverse_prefix()?;
        let root = cursor.edge();
        let bits = cursor.bits();
        let prefix = prefix.prefix(bits);
        Some(unsafe { iter::Prefix::new(guard, root, prefix, ..) })
    }

    // FIXME: support `Option` for min, max
    pub fn range<'k, R>(
        &mut self,
        range: R,
    ) -> Option<iter::Prefix<'k, 'g, K, V, R, Guard<'g, '_, K, V, S>>>
    where
        R: crate::raw::iter::Range<'k, K>,
    {
        let prefix = range.common_prefix();
        let guard = self.smr.guard(K::hazard(prefix));
        let mut cursor = unsafe { Cursor::<K, path::Discard>::new(self.raw.root(), prefix) };
        cursor.traverse_prefix()?;

        let root = cursor.edge();
        let bits = cursor.bits();
        let prefix = prefix.prefix(bits);

        Some(unsafe { iter::Prefix::new(guard, root, prefix, range) })
    }
}
