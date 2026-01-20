mod cursor;
mod hazard;
mod iter;
mod key;
mod value;

use core::ops::RangeFrom;
use core::ops::RangeFull;
use core::ops::RangeInclusive;
use core::sync::atomic::Ordering;

use polonius_the_crab::polonius;
use polonius_the_crab::polonius_return;

use crate::raw::cursor::path;
use crate::raw::edge;
use crate::raw::edge::Meta as _;
use crate::raw::key::Read as _;
use crate::raw::Edge;
use crate::raw::Frozen;
use crate::sequential;
use crate::stat;

use cursor::Cursor;
pub use iter::EntryIter;
pub use iter::Prefix;
pub use iter::ValueIter;
pub use key::Key;
pub use value::Value;

pub struct Map<K: Key, V: Value> {
    smr: hazard::Global<K::Prefix, V>,
    raw: sequential::Map<K, V>,
}

unsafe impl<K: Key, V: Value + Send + Sync> Sync for Map<K, V> {}

impl<K: crate::Key, V: Value> Default for Map<K, V> {
    fn default() -> Self {
        Self {
            smr: hazard::Global::default(),
            raw: sequential::Map::<K, V>::default(),
        }
    }
}

impl<K: Key, V: Value> Map<K, V> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_reclaim_threshold(reclaim_threshold: usize) -> Self {
        Self {
            smr: hazard::Global::with_reclaim_threshold(reclaim_threshold),
            raw: sequential::Map::<K, V>::default(),
        }
    }

    #[inline]
    pub fn pin(&self) -> MapRef<K, V> {
        MapRef {
            smr: self.smr.pin(),
            raw: &self.raw,
        }
    }

    #[inline]
    pub fn as_sequential(&mut self) -> &mut sequential::Map<K, V> {
        &mut self.raw
    }

    #[inline]
    pub fn set_membarrier(&mut self, membarrier: bool) {
        self.smr.set_membarrier(membarrier);
    }

    #[inline]
    pub fn reclaim(&mut self) {
        self.smr.reclaim(stat::Counter::FreeReclaim);
    }
}

pub struct MapRef<'g, K: Key, V: Value> {
    smr: hazard::Local<'g, K::Prefix, V>,
    raw: &'g sequential::Map<K, V>,
}

impl<'g, K, V> MapRef<'g, K, V>
where
    K: Key,
    V: Value + Send + Sync,
{
    #[inline]
    pub fn enable_membarrier(&self) {
        self.smr.enable_membarrier();
    }

    #[inline]
    pub fn get(&mut self, key: K::Borrow<'_>) -> Option<V::SharedGuard<'g, '_, K::Prefix>> {
        let reader = K::Read::from(key);
        Cursor::<K, V, path::Discard>::new(&mut self.smr, self.raw.root(), reader).traverse_get()
    }

    #[inline]
    pub fn update(
        &mut self,
        key: K::Borrow<'_>,
        value: V,
    ) -> Result<V::OwnedGuard<'g, '_, K::Prefix>, V> {
        let value = value.into_raw();
        let (old, present) = unsafe {
            self.get_and_update_with(key, &mut |old| Some(old.with_value(value)), &mut |_| ())
        };
        validate_eq!(old.is_some(), present);

        match old {
            Some(guard) => Ok(guard),
            None => Err(unsafe { V::from_raw(value) }),
        }
    }

    #[inline]
    pub fn update_with<F>(
        &mut self,
        key: K::Borrow<'_>,
        mut with: F,
    ) -> (Option<V::OwnedGuard<'g, '_, K::Prefix>>, bool)
    where
        F: FnMut(V::Borrow<'_>) -> Option<V>,
    {
        unsafe {
            self.get_and_update_with(
                key,
                &mut |old| {
                    let old_value = V::borrow_from_raw(old.into_raw());
                    let new_value = V::into_raw(with(old_value)?);
                    Some(old.with_value(new_value))
                },
                &mut |new| {
                    drop(V::from_raw(new));
                },
            )
        }
    }

    #[inline]
    pub fn remove(&mut self, key: K::Borrow<'_>) -> Option<V::OwnedGuard<'g, '_, K::Prefix>> {
        let (old, present) =
            unsafe { self.get_and_update_with(key, &mut |_| Some(Edge::DEFAULT), &mut |_| ()) };
        validate_eq!(old.is_some(), present);
        old
    }

    #[inline]
    pub fn remove_with<F>(
        &mut self,
        key: K::Borrow<'_>,
        mut with: F,
    ) -> (Option<V::OwnedGuard<'g, '_, K::Prefix>>, bool)
    where
        F: FnMut(V::Borrow<'_>) -> bool,
    {
        unsafe {
            self.get_and_update_with(
                key,
                &mut |old| {
                    let old_value = V::borrow_from_raw(old.into_raw());
                    with(old_value).then_some(Edge::DEFAULT)
                },
                &mut |_| (),
            )
        }
    }

    #[inline]
    unsafe fn get_and_update_with<A, D>(
        &mut self,
        key: K::Borrow<'_>,
        allocate: &mut A,
        deallocate: &mut D,
    ) -> (Option<V::OwnedGuard<'g, '_, K::Prefix>>, bool)
    where
        A: FnMut(ribbit::Packed<Edge<K::Edge>>) -> Option<ribbit::Packed<Edge<K::Edge>>>,
        D: FnMut(u64),
    {
        let mut map = self;

        // Cursed workaround for:
        // https://github.com/rust-lang/rust/issues/54663
        polonius!(
            |map| -> (Option<V::OwnedGuard<'g, 'polonius, K::Prefix>>, bool) {
                if let Ok(old) = map.get_and_update_with_optimistic(key, allocate, deallocate) {
                    polonius_return!(old);
                }
            }
        );

        map.get_and_update_with_pessimistic(key, allocate, deallocate)
    }

    #[inline]
    unsafe fn get_and_update_with_optimistic<A, D>(
        &mut self,
        key: K::Borrow<'_>,
        allocate: &mut A,
        deallocate: &mut D,
    ) -> Result<(Option<V::OwnedGuard<'g, '_, K::Prefix>>, bool), ()>
    where
        A: FnMut(ribbit::Packed<Edge<K::Edge>>) -> Option<ribbit::Packed<Edge<K::Edge>>>,
        D: FnMut(u64),
    {
        self.get_and_update_with_impl::<path::Discard, _, _>(key, allocate, deallocate)
    }

    #[cold]
    unsafe fn get_and_update_with_pessimistic<A, D>(
        &mut self,
        key: K::Borrow<'_>,
        allocate: &mut A,
        deallocate: &mut D,
    ) -> (Option<V::OwnedGuard<'g, '_, K::Prefix>>, bool)
    where
        A: FnMut(ribbit::Packed<Edge<K::Edge>>) -> Option<ribbit::Packed<Edge<K::Edge>>>,
        D: FnMut(u64),
    {
        self.get_and_update_with_impl::<path::Retain<_>, _, _>(key, allocate, deallocate)
            .unwrap()
    }

    #[inline]
    unsafe fn get_and_update_with_impl<'k, 'l, H, A, D>(
        &'l mut self,
        key: K::Borrow<'k>,
        allocate: &mut A,
        deallocate: &mut D,
    ) -> Result<(Option<V::OwnedGuard<'g, 'l, K::Prefix>>, bool), H::PopError>
    where
        H: path::History<'k, 'g, K>,
        A: FnMut(ribbit::Packed<Edge<K::Edge>>) -> Option<ribbit::Packed<Edge<K::Edge>>>,
        D: FnMut(u64),
    {
        let reader = K::Read::from(key);
        let mut cursor = Cursor::<K, V, H>::new(&mut self.smr, self.raw.root(), reader);

        loop {
            let old = match cursor.traverse_update() {
                None => return Ok((None, false)),
                Some(Ok(old)) => old,
                Some(Err(Frozen)) => {
                    cursor.freeze()?;
                    continue;
                }
            };

            validate!(old.meta().is_value());

            let new = match allocate(old) {
                Some(new) => new,
                None => return Ok((None, true)),
            };

            match cursor.edge().compare_exchange_packed(
                old,
                new,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    let guard = old
                        .as_value()
                        .map(|value| unsafe { V::guard_owned(cursor.into_guard(), value) });
                    return Ok((guard, true));
                }
                Err(_) => {
                    deallocate(new.into_raw());
                }
            }
        }
    }

    #[inline]
    pub fn upsert(
        &mut self,
        key: K::Borrow<'_>,
        value: V,
    ) -> Option<V::OwnedGuard<'g, '_, K::Prefix>> {
        let value = value.into_raw();
        let mut map = &mut *self;

        if !cfg!(feature = "opt-no-path") {
            // Cursed workaround for:
            // https://github.com/rust-lang/rust/issues/54663
            polonius!(|map| -> Option<V::OwnedGuard<'g, 'polonius, K::Prefix>> {
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
    ) -> Option<V::OwnedGuard<'g, '_, K::Prefix>>
    where
        F: FnMut(Option<V::Borrow<'_>>) -> V,
    {
        let mut map = &mut *self;

        // Cursed workaround for:
        // https://github.com/rust-lang/rust/issues/54663
        polonius!(|map| -> Option<V::OwnedGuard<'g, 'polonius, K::Prefix>> {
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
    ) -> Result<Option<V::OwnedGuard<'g, '_, K::Prefix>>, ()>
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
    ) -> Option<V::OwnedGuard<'g, '_, K::Prefix>>
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
    ) -> Result<Option<V::OwnedGuard<'g, '_, K::Prefix>>, H::PopError>
    where
        H: path::History<'k, 'g, K>,
        A: FnMut(Option<V::Borrow<'_>>) -> u64,
        D: FnMut(u64),
    {
        let reader = K::Read::from(key);
        let mut cursor = Cursor::<_, _, H>::new(&mut self.smr, self.raw.root(), reader);
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
                        Ok(old) => {
                            stat::increment(op);
                            if op.is_retire() {
                                unsafe {
                                    cursor.retire(old);
                                }
                            } else if matches!(op, crate::raw::Smo::CreateNode) {
                                return Ok(old.as_value().map(|value| unsafe {
                                    V::guard_owned(cursor.into_guard(), value)
                                }));
                            } else {
                                return Ok(None);
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
                Err(Frozen) => cursor.freeze()?,
            }
        }
    }

    /// Returns whether `key` was previously present in the tree.
    #[inline]
    pub fn get_or_insert(
        &mut self,
        key: K::Borrow<'_>,
        value: V,
    ) -> (V::SharedGuard<'g, '_, K::Prefix>, bool) {
        self.get_or_insert_with(key, || value)
    }

    /// Returns whether `key` was previously present in the tree.
    #[inline]
    pub fn get_or_insert_with<F>(
        &mut self,
        key: K::Borrow<'_>,
        with: F,
    ) -> (V::SharedGuard<'g, '_, K::Prefix>, bool)
    where
        F: FnOnce() -> V,
    {
        let mut map = &mut *self;
        let mut with = Thunk::new(|| V::into_raw(with()));

        // Cursed workaround for:
        // https://github.com/rust-lang/rust/issues/54663
        polonius!(|map| -> (V::SharedGuard<'g, 'polonius, K::Prefix>, bool) {
            if let Ok(old) = unsafe { map.get_or_insert_with_optimistic(key, &mut with) } {
                polonius_return!(old);
            }
        });

        unsafe { map.get_or_insert_with_pessimistic(key, &mut with) }
    }

    #[inline]
    unsafe fn get_or_insert_with_optimistic<'l, 'k, F>(
        &'l mut self,
        key: K::Borrow<'k>,
        with: &mut Thunk<F>,
    ) -> Result<(V::SharedGuard<'g, 'l, K::Prefix>, bool), ()>
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
    ) -> (V::SharedGuard<'g, '_, K::Prefix>, bool)
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
    ) -> Result<(V::SharedGuard<'g, '_, K::Prefix>, bool), H::PopError>
    where
        H: path::History<'k, 'g, K>,
        F: FnOnce() -> u64,
    {
        let reader = K::Read::from(key);
        let mut _cursor =
            Cursor::<'k, 'g, '_, _, _, H>::new(&mut self.smr, self.raw.root(), reader);

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

    pub fn all(&mut self) -> iter::Prefix<'static, 'g, '_, K, V, RangeFull> {
        let cursor =
            Cursor::<_, _, path::Discard>::new(&mut self.smr, self.raw.root(), K::Read::default());

        unsafe { iter::Prefix::new(K::Read::default(), cursor, ..) }
    }

    pub fn prefix<'k>(
        &mut self,
        prefix: impl Into<K::Read<'k>>,
    ) -> Option<iter::Prefix<'k, 'g, '_, K, V, RangeFull>> {
        let prefix = prefix.into();
        let mut cursor = Cursor::<_, _, path::Discard>::new(&mut self.smr, self.raw.root(), prefix);
        cursor.traverse_prefix()?;
        Some(unsafe { iter::Prefix::new(prefix, cursor, ..) })
    }

    // FIXME: support `Option` for min, max
    pub fn range<'k>(
        &mut self,
        min: impl Into<K::Read<'k>>,
        max: impl Into<K::Read<'k>>,
    ) -> Option<iter::Prefix<'k, 'g, '_, K, V, RangeInclusive<K::Read<'k>>>> {
        let min = min.into();
        let max = max.into();
        let prefix = min.common_prefix(max);
        let mut cursor = Cursor::<_, _, path::Discard>::new(&mut self.smr, self.raw.root(), prefix);
        cursor.traverse_prefix()?;
        Some(unsafe { iter::Prefix::new(prefix, cursor, min..=max) })
    }

    // FIXME: replace with generic range
    pub fn scan<'k>(
        &mut self,
        min: impl Into<K::Read<'k>>,
    ) -> iter::Prefix<'k, 'g, '_, K, V, RangeFrom<K::Read<'k>>> {
        let min = min.into();
        let cursor =
            Cursor::<_, _, path::Discard>::new(&mut self.smr, self.raw.root(), K::Read::default());
        unsafe { iter::Prefix::new(K::Read::default(), cursor, min..) }
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
