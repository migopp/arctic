mod iter;
mod key;
pub mod smr;
mod value;

use core::ops::ControlFlow;
use core::ops::RangeFull;
use core::sync::atomic::Ordering;

use polonius_the_crab::exit_polonius;
use polonius_the_crab::polonius;
use polonius_the_crab::polonius_return;
use smr::Guard as _;
use smr::Local as _;

use crate::raw::cursor::path;
use crate::raw::edge;
use crate::raw::edge::Meta as _;
use crate::raw::key::Read as _;
use crate::raw::Cursor;
use crate::raw::Edge;
use crate::raw::Frozen;
use crate::raw::Smo;
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

pub enum Update<'g, 'l, K, V, S = smr::Hazard<<K as Key>::Prefix, V>>
where
    K: Key,
    V: Value,
    S: Smr<K::Prefix, V> + 'g,
    S::Local<'g>: 'l,
{
    Absent {
        initial: Option<V>,
    },
    Success {
        old: Owned<'g, 'l, K, V, S>,
    },
    Failure {
        old: Shared<'g, 'l, K, V, S>,
        new: Option<V>,
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
        match self.update_with(key, None, |_, _| ControlFlow::Continue(None)) {
            Update::Absent { .. } => None,
            Update::Success { old } => Some(old),
            Update::Failure { .. } => unreachable!(),
        }
    }

    #[inline]
    pub fn remove_with<F>(&mut self, key: K::Borrow<'_>, mut with: F) -> Update<'g, '_, K, V, S>
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
        match self.update_with(key, Some(value), |_, new| ControlFlow::Continue(new)) {
            Update::Absent {
                initial: Some(initial),
            } => Err(initial),
            Update::Success { old } => Ok(old),
            Update::Absent { initial: None } | Update::Failure { .. } => unreachable!(),
        }
    }

    #[inline]
    pub fn update_with<F>(
        &mut self,
        key: K::Borrow<'_>,
        initial: Option<V>,
        mut update: F,
    ) -> Update<'g, '_, K, V, S>
    where
        F: FnMut(V::Borrow<'_>, Option<V>) -> ControlFlow<(), Option<V>>,
    {
        let mut map = self;

        // Cursed workaround for:
        // https://github.com/rust-lang/rust/issues/54663
        let initial = polonius!(|map| -> Update<'g, 'polonius, K, V, S> {
            match map.update_with_optimistic(key, initial, &mut update) {
                Ok(update) => polonius_return!(update),
                Err(initial) => exit_polonius!(initial),
            }
        });

        map.update_with_pessimistic(key, initial, update)
    }

    #[inline]
    fn update_with_optimistic<F>(
        &mut self,
        key: K::Borrow<'_>,
        initial: Option<V>,
        update: F,
    ) -> Result<Update<'g, '_, K, V, S>, Option<V>>
    where
        F: FnMut(V::Borrow<'_>, Option<V>) -> ControlFlow<(), Option<V>>,
    {
        self.update_with_impl::<path::Discard, _>(key, initial, update)
    }

    #[cold]
    fn update_with_pessimistic<F>(
        &mut self,
        key: K::Borrow<'_>,
        initial: Option<V>,
        update: F,
    ) -> Update<'g, '_, K, V, S>
    where
        F: FnMut(V::Borrow<'_>, Option<V>) -> ControlFlow<(), Option<V>>,
    {
        match self.update_with_impl::<path::Retain<_>, _>(key, initial, update) {
            Ok(update) => update,
            Err(_) => unreachable!(),
        }
    }

    #[inline]
    fn update_with_impl<'k, H, F>(
        &mut self,
        key: K::Borrow<'k>,
        mut initial: Option<V>,
        mut update: F,
    ) -> Result<Update<'g, '_, K, V, S>, Option<V>>
    where
        H: path::History<'k, 'g, K>,
        F: FnMut(V::Borrow<'_>, Option<V>) -> ControlFlow<(), Option<V>>,
    {
        let reader = K::Read::from(key);
        let mut guard = self.smr.guard(K::hazard(reader));
        let mut cursor = unsafe { Cursor::<_, H>::new(self.raw.root(), reader) };

        loop {
            let old = match cursor.traverse_update() {
                None => return Ok(Update::Absent { initial }),
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
                ControlFlow::Break(()) => {
                    return Ok(Update::Failure {
                        old: unsafe { V::share(guard, old.into_raw()) },
                        new: initial,
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
        let value = value.into_raw();
        let mut map = &mut *self;

        if !cfg!(feature = "opt-no-path") {
            // Cursed workaround for:
            // https://github.com/rust-lang/rust/issues/54663
            polonius!(|map| -> Option<Owned<'g, 'polonius, K, V, S>> {
                if let Ok(old) =
                    unsafe { map.upsert_with_optimistic(key, &mut |_| value, &mut |_| ()) }
                {
                    polonius_return!(old);
                }
            });
        }

        unsafe { map.upsert_with_pessimistic(key, &mut |_| value, &mut |_| ()) }
    }

    #[inline]
    pub fn upsert_with<F>(
        &mut self,
        key: K::Borrow<'_>,
        mut with: F,
    ) -> Option<Owned<'g, '_, K, V, S>>
    where
        F: FnMut(Option<V::Borrow<'_>>) -> V,
    {
        let mut map = &mut *self;

        // Cursed workaround for:
        // https://github.com/rust-lang/rust/issues/54663
        polonius!(|map| -> Option<Owned<'g, 'polonius, K, V, S>> {
            if let Ok(old) = unsafe {
                map.upsert_with_optimistic(key, &mut |old| with(old).into_raw(), &mut |raw| {
                    drop(V::from_raw(raw))
                })
            } {
                polonius_return!(old);
            }
        });

        unsafe {
            map.upsert_with_pessimistic(key, &mut |old| with(old).into_raw(), &mut |raw| {
                drop(V::from_raw(raw))
            })
        }
    }

    #[inline]
    unsafe fn upsert_with_optimistic<A, D>(
        &mut self,
        key: K::Borrow<'_>,
        allocate: &mut A,
        deallocate: &mut D,
    ) -> Result<Option<Owned<'g, '_, K, V, S>>, ()>
    where
        A: FnMut(Option<V::Borrow<'_>>) -> u64,
        D: FnMut(u64),
    {
        self.upsert_with_impl::<path::Discard, _, _>(key, allocate, deallocate)
    }

    #[cold]
    unsafe fn upsert_with_pessimistic<A, D>(
        &mut self,
        key: K::Borrow<'_>,
        allocate: &mut A,
        deallocate: &mut D,
    ) -> Option<Owned<'g, '_, K, V, S>>
    where
        A: FnMut(Option<V::Borrow<'_>>) -> u64,
        D: FnMut(u64),
    {
        stat::increment(stat::Counter::InsertPessimistic);
        unsafe {
            self.upsert_with_impl::<path::Retain<_>, _, _>(key, allocate, deallocate)
                .unwrap()
        }
    }

    // Note: the reason we need a `deallocate` function is to share this common
    // logic between (a) insert operations that insert one value unconditionally,
    // and don't need to allocate/deallocate based on the previous value, and
    // (b) insert operations that do depend on the previous value, and need to
    // be deallocated.
    #[inline]
    unsafe fn upsert_with_impl<'k, H, A, D>(
        &mut self,
        key: K::Borrow<'k>,
        allocate: &mut A,
        // FIXME
        deallocate: &mut D,
    ) -> Result<Option<Owned<'g, '_, K, V, S>>, H::PopError>
    where
        H: path::History<'k, 'g, K>,
        A: FnMut(Option<V::Borrow<'_>>) -> u64,
        D: FnMut(u64),
    {
        let reader = K::Read::from(key);
        let mut guard = self.smr.guard(K::hazard(reader));
        let mut cursor = Cursor::<_, H>::new(self.raw.root(), reader);
        // FXIME
        let value = allocate(None);

        loop {
            match cursor.traverse_upsert(value) {
                Ok((op, old, new)) => {
                    validate!(!old.meta().is_frozen());

                    match cursor.edge().compare_exchange_packed(
                        old,
                        new,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    ) {
                        Ok(old_) => {
                            stat::increment(op);

                            match old_.child() {
                                _ if op == Smo::ExpandEdge => return Ok(None),
                                None => {
                                    validate_eq!(op, Smo::CreateNode);
                                    return Ok(None);
                                }
                                Some(edge::Child::Value(value)) => {
                                    validate_eq!(op, Smo::CreateNode);
                                    return Ok(Some(unsafe { V::own(guard, value) }));
                                }
                                Some(edge::Child::Node(node)) => {
                                    validate!(op.is_retire());
                                    unsafe {
                                        guard.retire_node(cursor.bits(), node);
                                    }
                                }
                            }
                        }
                        Err(_) => {
                            // Does not go through SMR because `new` is still thread-local
                            if op.is_allocate() {
                                if let Some(edge::Child::Node(node)) = new.child() {
                                    unsafe {
                                        node.deallocate(stat::Counter::FreeConflict);
                                    }
                                }
                            } else if op.is_allocate_recursive() {
                                new.deallocate_recursive_unchecked(
                                    &mut *deallocate,
                                    stat::Counter::FreeConflict,
                                )
                            }
                        }
                    }
                }
                Err(Frozen) => {
                    if let Some(node) = cursor.freeze()? {
                        guard.retire_node(cursor.bits(), node);
                    }
                }
            }
        }
    }

    /// Returns whether `key` was previously present in the tree.
    #[inline]
    pub fn get_or_insert(
        &mut self,
        key: K::Borrow<'_>,
        value: V,
    ) -> (Shared<'g, '_, K, V, S>, bool) {
        self.get_or_insert_with(key, || value)
    }

    /// Returns whether `key` was previously present in the tree.
    #[inline]
    pub fn get_or_insert_with<F>(
        &mut self,
        key: K::Borrow<'_>,
        with: F,
    ) -> (Shared<'g, '_, K, V, S>, bool)
    where
        F: FnOnce() -> V,
    {
        let mut map = &mut *self;
        let mut with = Thunk::new(|| V::into_raw(with()));

        // Cursed workaround for:
        // https://github.com/rust-lang/rust/issues/54663
        polonius!(|map| -> (Shared<'g, 'polonius, K, V, S>, bool) {
            if let Ok(old) = unsafe { map.get_or_insert_with_optimistic(key, &mut with) } {
                polonius_return!(old);
            }
        });

        unsafe { map.get_or_insert_with_pessimistic(key, &mut with) }
    }

    #[inline]
    unsafe fn get_or_insert_with_optimistic<'k, F>(
        &mut self,
        key: K::Borrow<'k>,
        with: &mut Thunk<F>,
    ) -> Result<(Shared<'g, '_, K, V, S>, bool), ()>
    where
        F: FnOnce() -> u64,
    {
        self.get_or_insert_with_impl::<path::Discard, _>(key, with)
    }

    #[cold]
    unsafe fn get_or_insert_with_pessimistic<'k, F>(
        &mut self,
        key: K::Borrow<'k>,
        with: &mut Thunk<F>,
    ) -> (Shared<'g, '_, K, V, S>, bool)
    where
        F: FnOnce() -> u64,
    {
        stat::increment(stat::Counter::GetOrInsertPessimistic);
        unsafe {
            self.get_or_insert_with_impl::<path::Retain<_>, _>(key, with)
                .unwrap()
        }
    }

    #[inline]
    unsafe fn get_or_insert_with_impl<'k, H, F>(
        &mut self,
        key: K::Borrow<'k>,
        _with: &mut Thunk<F>,
    ) -> Result<(Shared<'g, '_, K, V, S>, bool), H::PopError>
    where
        H: path::History<'k, 'g, K>,
        F: FnOnce() -> u64,
    {
        let reader = K::Read::from(key);
        let _guard = self.smr.guard(K::hazard(reader));
        let mut _cursor = unsafe { Cursor::<'k, 'g, _, H>::new(self.raw.root(), reader) };

        loop {
            todo!()
            // match cursor.traverse_or_insert() {
            //     Insert::Value { old, key } => match old.as_value() {
            //         Some(value) => {
            //             // Deallocate `with` if we evaluated it
            //             match with {
            //                 Thunk::Unevaluated(_) => (),
            //                 Thunk::Evaluated(value) => drop(unsafe { V::from_raw(*value) }),
            //             }
            //
            //             return Ok((unsafe { V::guard_shared(cursor.into_guard(), value) }, true));
            //         }
            //         // Fall through to freeze
            //         None if old.meta().is_frozen() => (),
            //         None => {
            //             let new_value = with.evaluate();
            //             let new = Edge::new_value(key, new_value);
            //
            //             if cursor
            //                 .edge()
            //                 .compare_exchange_packed(old, new, Ordering::AcqRel, Ordering::Relaxed)
            //                 .is_ok()
            //             {
            //                 return Ok((
            //                     unsafe { V::guard_shared(cursor.into_guard(), new_value) },
            //                     false,
            //                 ));
            //             }
            //
            //             continue;
            //         }
            //     },
            //     Insert::Smo { op, old, new } => {
            //         validate!(!old.meta().is_frozen());
            //
            //         match cursor.edge().compare_exchange_packed(
            //             old,
            //             new,
            //             Ordering::AcqRel,
            //             Ordering::Acquire,
            //         ) {
            //             Ok(_) => {
            //                 stat::increment(op);
            //                 if op.is_retire() {
            //                     unsafe {
            //                         cursor.retire(old);
            //                     }
            //                 }
            //             }
            //             Err(_) => {
            //                 // Does not go through SMR because `new` is still thread-local
            //                 if op.is_allocate() {
            //                     if let Some(edge::Child::Node(node)) = new.child() {
            //                         unsafe {
            //                             node.deallocate_unchecked(stat::Counter::FreeConflict);
            //                         }
            //                     }
            //                 }
            //             }
            //         }
            //
            //         continue;
            //     }
            //     Insert::Frozen => (),
            // }
            //
            // cursor.freeze()?;
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

enum Thunk<F> {
    Unevaluated(F),
    Evaluated(u64),
}

impl<F> Thunk<F> {
    fn new(with: F) -> Self {
        Self::Unevaluated(with)
    }
}

impl<F> Thunk<F>
where
    F: FnOnce() -> u64,
{
    fn evaluate(&mut self) -> u64 {
        let thunk = core::mem::replace(self, Self::Evaluated(0));
        let value = match thunk {
            Self::Unevaluated(with) => with(),
            Self::Evaluated(value) => value,
        };
        *self = Self::Evaluated(value);
        value
    }
}
