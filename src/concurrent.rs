mod cursor;
mod hazard;
mod iter;
pub(crate) mod key;
pub(crate) mod value;

use core::ops::RangeFull;
use core::ops::RangeInclusive;
use core::sync::atomic::Ordering;

use polonius_the_crab::polonius;
use polonius_the_crab::polonius_return;

use crate::iter::Order;
use crate::raw::cursor::Insert;
use crate::raw::edge;
use crate::raw::edge::Meta as _;
use crate::raw::key::Read as _;
use crate::raw::Edge;
use crate::sequential;
use crate::stat;
use crate::Key;
use crate::Value;

pub struct Map<K: Key, V: Value> {
    smr: hazard::Global<V>,
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
}

pub struct MapRef<'g, K: Key, V: Value> {
    smr: hazard::Local<'g, V>,
    raw: &'g sequential::Map<K, V>,
}

impl<'g, K, V> MapRef<'g, K, V>
where
    K: Key,
    V: Value + Send + Sync,
{
    #[inline]
    pub fn get(&mut self, key: <K as Key>::Borrow<'_>) -> Option<V::SharedGuard<'g, '_>> {
        cursor::Point::<K, V, _>::get(&mut self.smr, self.raw.root(), K::Read::from(key))
    }

    #[inline]
    pub fn update(
        &mut self,
        key: <K as Key>::Borrow<'_>,
        value: V,
    ) -> Option<V::OwnedGuard<'g, '_>> {
        let value = value.into_raw();
        unsafe {
            self.get_and_update_with(key, &mut |old| Some(old.with_value(value)), &mut |_| ())
                .into_inner()
        }
    }

    #[inline]
    pub fn update_with<F>(
        &mut self,
        key: <K as Key>::Borrow<'_>,
        mut with: F,
    ) -> Effect<Option<V::OwnedGuard<'g, '_>>>
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
    pub fn remove(&mut self, key: <K as Key>::Borrow<'_>) -> Option<V::OwnedGuard<'g, '_>> {
        unsafe { self.get_and_update_with(key, &mut |_| Some(Edge::DEFAULT), &mut |_| ()) }
            .into_inner()
    }

    #[inline]
    pub fn remove_with<F>(
        &mut self,
        key: <K as Key>::Borrow<'_>,
        mut with: F,
    ) -> Effect<Option<V::OwnedGuard<'g, '_>>>
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
        key: <K as Key>::Borrow<'_>,
        allocate: &mut A,
        deallocate: &mut D,
    ) -> Effect<Option<V::OwnedGuard<'g, '_>>>
    where
        A: FnMut(ribbit::Packed<Edge<K::Edge>>) -> Option<ribbit::Packed<Edge<K::Edge>>>,
        D: FnMut(u64),
    {
        let mut map = self;

        // Cursed workaround for:
        // https://github.com/rust-lang/rust/issues/54663
        polonius!(|map| -> Effect<Option<V::OwnedGuard<'g, 'polonius>>> {
            if let Ok(old) = map.get_and_update_with_optimistic(key, allocate, deallocate) {
                polonius_return!(old);
            }
        });

        map.get_and_update_with_pessimistic(key, allocate, deallocate)
    }

    #[inline]
    unsafe fn get_and_update_with_optimistic<A, D>(
        &mut self,
        key: <K as Key>::Borrow<'_>,
        allocate: &mut A,
        deallocate: &mut D,
    ) -> Result<Effect<Option<V::OwnedGuard<'g, '_>>>, ()>
    where
        A: FnMut(ribbit::Packed<Edge<K::Edge>>) -> Option<ribbit::Packed<Edge<K::Edge>>>,
        D: FnMut(u64),
    {
        self.get_and_update_with_impl::<cursor::path::Discard, _, _>(key, allocate, deallocate)
    }

    #[cold]
    unsafe fn get_and_update_with_pessimistic<A, D>(
        &mut self,
        key: <K as Key>::Borrow<'_>,
        allocate: &mut A,
        deallocate: &mut D,
    ) -> Effect<Option<V::OwnedGuard<'g, '_>>>
    where
        A: FnMut(ribbit::Packed<Edge<K::Edge>>) -> Option<ribbit::Packed<Edge<K::Edge>>>,
        D: FnMut(u64),
    {
        self.get_and_update_with_impl::<cursor::path::Retain<_>, _, _>(key, allocate, deallocate)
            .unwrap()
    }

    #[inline]
    unsafe fn get_and_update_with_impl<'l, 'k, H, A, D>(
        &'l mut self,
        key: <K as Key>::Borrow<'k>,
        allocate: &mut A,
        deallocate: &mut D,
    ) -> Result<Effect<Option<V::OwnedGuard<'g, 'l>>>, H::PopError>
    where
        H: cursor::path::History<'g, 'k, K>,
        A: FnMut(ribbit::Packed<Edge<K::Edge>>) -> Option<ribbit::Packed<Edge<K::Edge>>>,
        D: FnMut(u64),
    {
        let reader = K::Read::from(key);
        let mut cursor = cursor::Point::<K, V, H>::new(&mut self.smr, self.raw.root(), reader);

        loop {
            let old = match cursor.traverse_exact() {
                None => return Ok(Effect::Read(None)),
                Some(Ok(old)) => old,
                Some(Err(())) => {
                    cursor.freeze()?;
                    continue;
                }
            };

            validate!(old.meta().is_value());

            let new = match allocate(old) {
                Some(new) => new,
                None => return Ok(Effect::Read(None)),
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
                    return Ok(Effect::Write(guard));
                }
                Err(_) => {
                    deallocate(new.into_raw());
                }
            }
        }
    }

    #[inline]
    pub fn insert(
        &mut self,
        key: <K as Key>::Borrow<'_>,
        value: V,
    ) -> Option<V::OwnedGuard<'g, '_>> {
        let value = value.into_raw();
        let mut map = &mut *self;

        // Cursed workaround for:
        // https://github.com/rust-lang/rust/issues/54663
        polonius!(|map| -> Option<V::OwnedGuard<'g, 'polonius>> {
            if let Ok(old) = unsafe { map.insert_with_optimistic(key, &mut |_| value, &mut |_| ()) }
            {
                polonius_return!(old);
            }
        });

        unsafe { map.insert_with_pessimistic(key, &mut |_| value, &mut |_| ()) }
    }

    #[inline]
    pub fn insert_with<F>(
        &mut self,
        key: <K as Key>::Borrow<'_>,
        mut with: F,
    ) -> Option<V::OwnedGuard<'g, '_>>
    where
        F: FnMut(Option<V::Borrow<'_>>) -> V,
    {
        let mut map = &mut *self;

        // Cursed workaround for:
        // https://github.com/rust-lang/rust/issues/54663
        polonius!(|map| -> Option<V::OwnedGuard<'g, 'polonius>> {
            if let Ok(old) = unsafe {
                map.insert_with_optimistic(key, &mut |old| with(old).into_raw(), &mut |raw| {
                    drop(V::from_raw(raw))
                })
            } {
                polonius_return!(old);
            }
        });

        unsafe {
            map.insert_with_pessimistic(key, &mut |old| with(old).into_raw(), &mut |raw| {
                drop(V::from_raw(raw))
            })
        }
    }

    #[inline]
    unsafe fn insert_with_optimistic<A, D>(
        &mut self,
        key: <K as Key>::Borrow<'_>,
        allocate: &mut A,
        deallocate: &mut D,
    ) -> Result<Option<V::OwnedGuard<'g, '_>>, ()>
    where
        A: FnMut(Option<V::Borrow<'_>>) -> u64,
        D: FnMut(u64),
    {
        self.insert_with_impl::<cursor::path::Discard, _, _>(key, allocate, deallocate)
    }

    #[cold]
    unsafe fn insert_with_pessimistic<A, D>(
        &mut self,
        key: <K as Key>::Borrow<'_>,
        allocate: &mut A,
        deallocate: &mut D,
    ) -> Option<V::OwnedGuard<'g, '_>>
    where
        A: FnMut(Option<V::Borrow<'_>>) -> u64,
        D: FnMut(u64),
    {
        stat::increment(stat::Counter::InsertPessimistic);
        unsafe {
            self.insert_with_impl::<cursor::path::Retain<_>, _, _>(key, allocate, deallocate)
                .unwrap()
        }
    }

    // Note: the reason we need a `deallocate` function is to share this common
    // logic between (a) insert operations that insert one value unconditionally,
    // and don't need to allocate/deallocate based on the previous value, and
    // (b) insert operations that do depend on the previous value, and need to
    // be deallocated.
    #[inline]
    unsafe fn insert_with_impl<'k, H, A, D>(
        &mut self,
        key: <K as Key>::Borrow<'k>,
        allocate: &mut A,
        deallocate: &mut D,
    ) -> Result<Option<V::OwnedGuard<'g, '_>>, H::PopError>
    where
        H: cursor::path::History<'g, 'k, K>,
        A: FnMut(Option<V::Borrow<'_>>) -> u64,
        D: FnMut(u64),
    {
        let reader = K::Read::from(key);
        let mut cursor = cursor::Point::<_, _, H>::new(&mut self.smr, self.raw.root(), reader);

        loop {
            match cursor.traverse_or_insert() {
                Insert::Value { old, key } if !old.meta().is_frozen() => {
                    let old_value = old.as_value().map(|raw| unsafe { V::borrow_from_raw(raw) });
                    let new_value = allocate(old_value);

                    if cursor
                        .edge()
                        .compare_exchange_packed(
                            old,
                            Edge::new_value(key, new_value),
                            Ordering::AcqRel,
                            Ordering::Relaxed,
                        )
                        .is_err()
                    {
                        deallocate(new_value);
                        continue;
                    }

                    return Ok(old
                        .as_value()
                        .map(|value| unsafe { V::guard_owned(cursor.into_guard(), value) }));
                }
                Insert::Smo { op, old, new } => {
                    validate!(!old.meta().is_frozen());

                    match cursor.edge().compare_exchange_packed(
                        old,
                        new,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    ) {
                        Ok(_) => {
                            stat::increment(op);
                            if op.is_retire() {
                                unsafe {
                                    cursor.retire(old);
                                }
                            }
                        }
                        Err(_) => {
                            // Does not go through SMR because `new` is still thread-local
                            if op.is_allocate() {
                                if let Some(edge::Child::Node(node)) = new.child() {
                                    unsafe {
                                        node.deallocate_unchecked(stat::Counter::FreeConflict);
                                    }
                                }
                            }
                        }
                    }
                }
                Insert::Frozen | Insert::Value { .. } => cursor.freeze()?,
            }
        }
    }

    #[inline]
    pub fn get_or_insert<F>(
        &mut self,
        key: <K as Key>::Borrow<'_>,
        value: V,
    ) -> Effect<V::SharedGuard<'g, '_>>
    where
        F: FnOnce() -> V,
    {
        self.get_or_insert_with(key, || value)
    }

    #[inline]
    pub fn get_or_insert_with<F>(
        &mut self,
        key: <K as Key>::Borrow<'_>,
        with: F,
    ) -> Effect<V::SharedGuard<'g, '_>>
    where
        F: FnOnce() -> V,
    {
        let mut map = &mut *self;
        let mut with = Thunk::new(|| V::into_raw(with()));

        // Cursed workaround for:
        // https://github.com/rust-lang/rust/issues/54663
        polonius!(|map| -> Effect<V::SharedGuard<'g, 'polonius>> {
            if let Ok(old) = unsafe { map.get_or_insert_with_optimistic(key, &mut with) } {
                polonius_return!(old);
            }
        });

        unsafe { map.get_or_insert_with_pessimistic(key, &mut with) }
    }

    #[inline]
    unsafe fn get_or_insert_with_optimistic<'l, 'k, F>(
        &'l mut self,
        key: <K as Key>::Borrow<'k>,
        with: &mut Thunk<F>,
    ) -> Result<Effect<V::SharedGuard<'g, 'l>>, ()>
    where
        F: FnOnce() -> u64,
    {
        self.get_or_insert_with_impl::<cursor::path::Discard, _>(key, with)
    }

    #[cold]
    unsafe fn get_or_insert_with_pessimistic<'l, 'k, F>(
        &'l mut self,
        key: <K as Key>::Borrow<'k>,
        with: &mut Thunk<F>,
    ) -> Effect<V::SharedGuard<'g, 'l>>
    where
        F: FnOnce() -> u64,
    {
        stat::increment(stat::Counter::GetOrInsertPessimistic);
        unsafe {
            self.get_or_insert_with_impl::<cursor::path::Retain<_>, _>(key, with)
                .unwrap()
        }
    }

    #[inline]
    unsafe fn get_or_insert_with_impl<'l, 'k, H, F>(
        &'l mut self,
        key: <K as Key>::Borrow<'k>,
        with: &mut Thunk<F>,
    ) -> Result<Effect<V::SharedGuard<'g, 'l>>, H::PopError>
    where
        H: cursor::path::History<'g, 'k, K>,
        F: FnOnce() -> u64,
    {
        let reader = K::Read::from(key);
        let mut cursor =
            cursor::Point::<'g, 'l, 'k, _, _, H>::new(&mut self.smr, self.raw.root(), reader);

        loop {
            match cursor.traverse_or_insert() {
                Insert::Value { old, key } => match old.as_value() {
                    Some(value) => {
                        // Deallocate `with` if we evaluated it
                        match with {
                            Thunk::Unevaluated(_) => (),
                            Thunk::Evaluated(value) => drop(unsafe { V::from_raw(*value) }),
                        }

                        return Ok(Effect::Write(unsafe {
                            V::guard_shared(cursor.into_guard(), value)
                        }));
                    }
                    // Fall through to freeze
                    None if old.meta().is_frozen() => (),
                    None => {
                        let new_value = with.evaluate();
                        let new = Edge::new_value(key, new_value);

                        if cursor
                            .edge()
                            .compare_exchange_packed(old, new, Ordering::AcqRel, Ordering::Relaxed)
                            .is_ok()
                        {
                            return Ok(Effect::Read(unsafe {
                                V::guard_shared(cursor.into_guard(), new_value)
                            }));
                        }

                        continue;
                    }
                },
                Insert::Smo { op, old, new } => {
                    validate!(!old.meta().is_frozen());

                    match cursor.edge().compare_exchange_packed(
                        old,
                        new,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    ) {
                        Ok(_) => {
                            stat::increment(op);
                            if op.is_retire() {
                                unsafe {
                                    cursor.retire(old);
                                }
                            }
                        }
                        Err(_) => {
                            // Does not go through SMR because `new` is still thread-local
                            if op.is_allocate() {
                                if let Some(edge::Child::Node(node)) = new.child() {
                                    unsafe {
                                        node.deallocate_unchecked(stat::Counter::FreeConflict);
                                    }
                                }
                            }
                        }
                    }

                    continue;
                }
                Insert::Frozen => (),
            }

            cursor.freeze()?;
        }
    }

    pub fn all(&mut self) -> iter::PrefixGuard<'g, '_, '_, K, V, RangeFull> {
        let cursor =
            cursor::Prefix::<_, _, cursor::path::Discard>::new_root(&mut self.smr, self.raw.root());

        iter::PrefixGuard::new(cursor, ..)
    }

    pub fn prefix<'k>(
        &mut self,
        prefix: impl Into<K::Read<'k>>,
    ) -> Option<iter::PrefixGuard<'g, '_, 'k, K, V, RangeFull>> {
        let prefix = prefix.into();
        let cursor = cursor::Prefix::<_, _, cursor::path::Discard>::new(
            &mut self.smr,
            self.raw.root(),
            prefix,
        )?;
        Some(iter::PrefixGuard::new(cursor, ..))
    }

    // FIXME: support `Option` for min, max
    pub fn range<'k>(
        &mut self,
        min: impl Into<K::Read<'k>>,
        max: impl Into<K::Read<'k>>,
    ) -> Option<iter::PrefixGuard<'g, '_, 'k, K, V, RangeInclusive<K::Read<'k>>>> {
        let min = min.into();
        let max = max.into();
        let cursor = cursor::Prefix::<_, _, cursor::path::Discard>::new(
            &mut self.smr,
            self.raw.root(),
            min.common_prefix(max),
        )?;
        Some(iter::PrefixGuard::new(cursor, min..=max))
    }

    pub fn prefix_optimistic<'l, O: Order>(
        &'l mut self,
        buffer: &'l mut Vec<(K::Write, u64)>,
        limit: usize,
        prefix: impl Into<K::Read<'l>>,
    ) -> Option<LinearizableGuard<'g, 'l, K, V>> {
        let guard = self.prefix(prefix)?;
        match Self::scan_optimistic::<_, O>(buffer, &guard, limit) {
            Ok(()) => Some(LinearizableGuard {
                guard: guard.guard_value(),
                buffer,
            }),
            Err(()) => todo!(),
        }
    }

    // pub fn prefix_pessimistic<'l, S: Sort>(
    //     &'l mut self,
    //     buffer: &'l mut Vec<(K::Write, u64)>,
    //     prefix: impl Into<K::Read<'l>>,
    // ) -> Option<LinearizableGuard<'g, 'l, K, V>> {
    //     let guard = self.prefix(prefix)?;
    //     Self::scan_pessimistic::<iter::Prefix, S>(buffer, guard)
    // }

    pub fn range_optimistic<'l, 'k, O: Order>(
        &'l mut self,
        buffer: &'l mut Vec<(K::Write, u64)>,
        limit: usize,
        min: impl Into<K::Read<'k>>,
        max: impl Into<K::Read<'k>>,
    ) -> Option<LinearizableGuard<'g, 'l, K, V>> {
        let guard = self.range(min, max)?;

        match Self::scan_optimistic::<_, O>(buffer, &guard, limit) {
            Ok(()) => Some(LinearizableGuard {
                guard: guard.guard_value(),
                buffer,
            }),
            Err(()) => todo!(),
        }
    }

    // pub fn range_pessimistic<'l, S: Sort>(
    //     &'l mut self,
    //     buffer: &'l mut Vec<(K::Write, u64)>,
    //     min: impl Into<K::Read<'l>>,
    //     max: impl Into<K::Read<'l>>,
    // ) -> Option<LinearizableGuard<'g, 'l, K, V>> {
    //     let min = min.into();
    //     let max = max.into();
    //     let cursor = cursor::Prefix::<_, (), _, cursor::path::Hybrid<_, _>>::new_range(
    //         &mut self.smr,
    //         self.raw.root(),
    //         min,
    //         max,
    //     )?;
    //     Self::scan_pessimistic::<iter::Range, S>(buffer, cursor, &(min, max))
    // }

    // fn scan_hybrid<'l, S, O>(
    //     buffer: &'l mut Vec<(K::Write, u64)>,
    //     guard: iter::Guard<'g, 'l, K::Read<'l>, (), V, S>,
    //     limit: usize,
    // ) -> Option<LinearizableGuard<'g, 'l, K, V>>
    // where
    //     S: Scan,
    //     O: Sort,
    // {
    //     match Self::scan_optimistic::<S, O>(buffer, &guard, limit) {
    //         Ok(()) => Some(LinearizableGuard {
    //             guard: guard.guard_value(),
    //             buffer,
    //         }),
    //         Err(()) => Self::scan_pessimistic::<S, O>(buffer, guard),
    //     }
    // }

    fn scan_optimistic<'k, R, O>(
        buffer: &mut Vec<(K::Write, u64)>,
        guard: &iter::PrefixGuard<'g, '_, 'k, K, V, R>,
        limit: usize,
    ) -> Result<(), ()>
    where
        R: crate::raw::iter::Range<K::Read<'k>>,
        O: Order,
    {
        guard
            .entries::<O>()
            .for_each_raw(|key, value| buffer.push((key.clone(), value)));

        for retry in 0..=limit {
            let mut dirty = false;
            let mut len = 0;

            guard.entries::<O>().for_each_raw(|new_key, new_value| {
                let index = len;
                len += 1;

                let old = match buffer.get_mut(index) {
                    // Fast path: no change
                    Some((old_key, old_value)) if old_key == new_key && *old_value == new_value => {
                        return;
                    }
                    old => old,
                };

                crate::cold();
                dirty = true;

                match old {
                    Some((old_key, old_value)) if old_key == new_key => {
                        *old_value = new_value;
                    }
                    Some((old_key, _)) if *old_key < *new_key => {
                        let high = buffer[len..]
                            .iter()
                            .position(|(key, _)| key >= new_key)
                            .map(|offset| len + offset)
                            .unwrap_or(buffer.len());
                        buffer.drain(index..high);
                        len = index;
                    }
                    None | Some(_) => {
                        buffer.insert(index, (new_key.clone(), new_value));
                    }
                };
            });

            if len == buffer.len() && !dirty {
                stat::record(stat::Record::RangeConflict, retry as u64);
                return Ok(());
            }

            validate!(buffer.len() <= len);
            buffer.truncate(len);
        }

        Err(())
    }

    // fn scan_pessimistic<'l, S, O>(
    //     buffer: &'l mut Vec<(K::Write, u64)>,
    //     guard: iter::Guard<'g, 'l, K::Read<'l>, (), V, S>,
    // ) -> Option<LinearizableGuard<'g, 'l, K, V>>
    // where
    //     S: Scan,
    //     O: Sort,
    // {
    //     Self::lock_prefix(&mut cursor)?;
    //
    //     S::scan::<_, _, _, O, _>(&cursor, arg, |key, value| buffer.push((key.clone(), value)));
    //     Self::unlock_prefix(&mut cursor);
    //
    //     Some(LinearizableGuard {
    //         guard: unsafe { V::downgrade_guard(cursor.into_guard()) },
    //         buffer,
    //     })
    // }

    fn lock_prefix<'k>(
        cursor: &mut cursor::Prefix<'g, '_, 'k, K, V, cursor::path::Hybrid<'g, 'k, K>>,
    ) -> Option<()> {
        let mut edge = cursor.edge().load_packed(Ordering::Relaxed);

        loop {
            // No need to lock value
            let Some(node) = edge.as_node() else {
                return Some(());
            };

            if edge.meta().is_frozen() || node.scan() {
                match cursor.wait_for_scan(stat::Counter::ScanScan) {
                    Ok(safe) if !edge.meta().is_frozen() => edge = safe,
                    Ok(_) | Err(()) => {
                        edge = cursor.freeze()?;
                        continue;
                    }
                }
            }

            match cursor.edge().compare_exchange_packed(
                edge,
                edge.with_node(node.with_scan(true)),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Some(()),
                Err(conflict) => {
                    core::hint::spin_loop();
                    edge = conflict;
                }
            }
        }
    }

    #[inline]
    fn unlock_prefix<'k>(
        cursor: &mut cursor::Prefix<'g, '_, 'k, K, V, cursor::path::Hybrid<'g, 'k, K>>,
    ) {
        let mut edge = cursor.edge().load_packed(Ordering::Relaxed);

        let Some(node) = edge.as_node() else { return };

        loop {
            validate!(node.scan());

            if edge.meta().is_frozen() {
                edge = match cursor.freeze() {
                    Some(edge) => edge,
                    None => unreachable!("Locked edge must be reachable"),
                };
                continue;
            }

            match cursor.edge().compare_exchange_packed(
                edge,
                edge.with_node(node.with_scan(false)),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(conflict) => {
                    core::hint::spin_loop();
                    edge = conflict;
                }
            }
        }
    }
}

/// Provides extra information about the effect of a fallible operation,
/// which may be useful for coordination.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Effect<T> {
    /// Effect is read-only, tree is not modified
    Read(T),
    /// Effect wrote to tree
    Write(T),
}

impl<T> Effect<T> {
    #[inline]
    pub fn is_read(&self) -> bool {
        matches!(self, Self::Read(_))
    }

    #[inline]
    pub fn is_write(&self) -> bool {
        matches!(self, Self::Write(_))
    }

    #[inline]
    pub fn into_inner(self) -> T {
        match self {
            Self::Read(inner) | Self::Write(inner) => inner,
        }
    }
}

impl<T> core::ops::Deref for Effect<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        match self {
            Effect::Read(inner) | Effect::Write(inner) => inner,
        }
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

pub struct LinearizableGuard<'g: 'l, 'l, K: Key, V: Value + 'g> {
    guard: V::LinearizableGuard<'g, 'l>,
    buffer: &'l mut Vec<(K::Write, u64)>,
}

impl<'g, 'l, K: Key, V: Value> LinearizableGuard<'g, 'l, K, V> {
    #[inline]
    pub fn drain(&mut self) -> LinearizableDrain<'g, 'l, '_, K, V> {
        LinearizableDrain {
            guard: &self.guard,
            iter: self.buffer.drain(..),
        }
    }
}

pub struct LinearizableDrain<'g: 'l, 'l, 'a, K: Key, V: Value + 'g> {
    guard: &'a V::LinearizableGuard<'g, 'l>,
    iter: std::vec::Drain<'a, (K::Write, u64)>,
}

impl<'g, 'l, 'a, K, V> Iterator for LinearizableDrain<'g, 'l, 'a, K, V>
where
    K: Key,
    V: Value + 'g,
    'g: 'l,
{
    type Item = (K, V::Borrow<'l>);
    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next().map(|(key, value)| {
            // FIXME: take ownership of key directly
            (unsafe { K::from_writer_unchecked(key) }, unsafe {
                V::guard_linearizable(self.guard, value)
            })
        })
    }
}
