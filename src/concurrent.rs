mod cursor;
mod hazard;
mod iter;
mod value;

use core::sync::atomic::Ordering;

use polonius_the_crab::polonius;
use polonius_the_crab::polonius_return;
use ribbit::atomic::Atomic128;

use crate::iter::Sort;
use crate::key;
use crate::key::Read as _;
use crate::raw::edge;
use crate::raw::Edge;
use crate::raw::Op;
use crate::sequential;
use crate::stat;
use crate::Key;
use iter::Scan;
pub use value::Value;

pub struct Map<K, V: Value> {
    smr: hazard::Global<V>,
    raw: sequential::Map<K, V>,
}

unsafe impl<K, V: Value + Send + Sync> Sync for Map<K, V> {}

impl<K, V: Value> Default for Map<K, V> {
    fn default() -> Self {
        Self {
            smr: hazard::Global::default(),
            raw: sequential::Map::<K, V>::default(),
        }
    }
}

impl<K, V: Value> Map<K, V> {
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

pub struct MapRef<'g, K, V: Value> {
    smr: hazard::Local<'g, V>,
    raw: &'g sequential::Map<K, V>,
}

impl<'g, K, V> MapRef<'g, K, V>
where
    K: Key,
    V: Value + Send + Sync,
{
    #[inline]
    pub fn get(&mut self, key: K::Borrow<'_>) -> Option<V::SharedGuard<'g, '_>> {
        cursor::Point::get(&mut self.smr, self.raw.root(), K::Read::from(key))
    }

    #[inline]
    pub fn update(&mut self, key: K::Borrow<'_>, value: V) -> Option<V::OwnedGuard<'g, '_>> {
        let value = u64::from(value.into_raw());
        unsafe { self.compare_exchange(key, |old| old.with_value(value)) }
    }

    #[inline]
    pub fn remove(&mut self, key: K::Borrow<'_>) -> Option<V::OwnedGuard<'g, '_>> {
        unsafe { self.compare_exchange(key, |_| Edge::DEFAULT) }
    }

    #[inline]
    unsafe fn compare_exchange<F>(
        &mut self,
        key: K::Borrow<'_>,
        mut exchange: F,
    ) -> Option<V::OwnedGuard<'g, '_>>
    where
        F: FnMut(ribbit::Packed<Edge<()>>) -> ribbit::Packed<Edge<()>>,
    {
        let mut map = self;

        // Cursed workaround for:
        // https://github.com/rust-lang/rust/issues/54663
        polonius!(|map| -> Option<V::OwnedGuard<'g, 'polonius>> {
            if let Ok(old) = map.compare_exchange_optimistic::<_>(key, &mut exchange) {
                polonius_return!(old);
            }
        });

        map.compare_exchange_pessimistic(key, exchange)
    }

    #[inline]
    unsafe fn compare_exchange_optimistic<F>(
        &mut self,
        key: K::Borrow<'_>,
        exchange: F,
    ) -> Result<Option<V::OwnedGuard<'g, '_>>, ()>
    where
        F: FnMut(ribbit::Packed<Edge<()>>) -> ribbit::Packed<Edge<()>>,
    {
        self.compare_exchange_impl::<cursor::path::Discard, _>(key, exchange)
    }

    #[cold]
    unsafe fn compare_exchange_pessimistic<F>(
        &mut self,
        key: K::Borrow<'_>,
        exchange: F,
    ) -> Option<V::OwnedGuard<'g, '_>>
    where
        F: FnMut(ribbit::Packed<Edge<()>>) -> ribbit::Packed<Edge<()>>,
    {
        self.compare_exchange_impl::<cursor::path::Retain<_, _>, _>(key, exchange)
            .unwrap()
    }

    /// # SAFETY
    ///
    /// Caller must guarantee that `exchange` removes the old value from the tree,
    /// or else we will duplicate ownership.
    #[inline]
    unsafe fn compare_exchange_impl<'k, H, F>(
        &mut self,
        key: K::Borrow<'k>,
        mut exchange: F,
    ) -> Result<Option<V::OwnedGuard<'g, '_>>, H::PopError>
    where
        H: cursor::path::History<'g, K::Read<'k>, ()>,
        F: FnMut(ribbit::Packed<Edge<()>>) -> ribbit::Packed<Edge<()>>,
    {
        let reader = K::Read::from(key);
        let mut cursor = cursor::Point::<_, (), _, H>::new(&mut self.smr, self.raw.root(), reader);

        loop {
            let old = match cursor.traverse_exact() {
                None => return Ok(None),
                Some(Ok(old)) => old,
                Some(Err(())) => {
                    cursor.freeze()?;
                    continue;
                }
            };

            if cursor
                .edge()
                .compare_exchange_packed(old, exchange(old), Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Ok(match old.as_value() {
                    Some(value) => Some(unsafe { V::guard_owned(cursor.into_guard(), value) }),
                    None => {
                        validate!(old.is_null());
                        None
                    }
                });
            }
        }
    }

    #[inline]
    pub fn insert(&mut self, key: K::Borrow<'_>, value: V) -> Option<V::OwnedGuard<'g, '_>> {
        let value = value.into_raw();
        let mut map = &mut *self;

        // Cursed workaround for:
        // https://github.com/rust-lang/rust/issues/54663
        polonius!(|map| -> Option<V::OwnedGuard<'g, 'polonius>> {
            if let Ok(old) = map.insert_optimistic(key, value) {
                polonius_return!(old);
            }
        });

        map.insert_pessimistic(key, value)
    }

    #[inline]
    fn insert_optimistic(
        &mut self,
        key: K::Borrow<'_>,
        value: u64,
    ) -> Result<Option<V::OwnedGuard<'g, '_>>, ()> {
        self.insert_impl::<cursor::path::Discard>(key, value)
    }

    #[cold]
    fn insert_pessimistic(
        &mut self,
        key: K::Borrow<'_>,
        value: u64,
    ) -> Option<V::OwnedGuard<'g, '_>> {
        stat::increment(stat::Counter::InsertPessimistic);
        self.insert_impl::<cursor::path::Retain<_, _>>(key, value)
            .unwrap()
    }

    #[inline]
    fn insert_impl<'k, H>(
        &mut self,
        key: K::Borrow<'k>,
        value: u64,
    ) -> Result<Option<V::OwnedGuard<'g, '_>>, H::PopError>
    where
        H: cursor::path::History<'g, K::Read<'k>, ()>,
    {
        let reader = K::Read::from(key);
        let mut cursor = cursor::Point::<_, (), _, H>::new(&mut self.smr, self.raw.root(), reader);

        loop {
            let (op, old, new) = match cursor.traverse_or_insert(value) {
                Ok(cas) => cas,
                Err(()) => {
                    cursor.freeze()?;
                    continue;
                }
            };

            validate!(!old.meta().is_frozen());

            match cursor.edge().compare_exchange_packed(
                old,
                new,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) if op == Op::Edge(crate::raw::edge::Op::Insert) => {
                    stat::increment(op);
                    return match old.as_value() {
                        Some(value) => {
                            Ok(Some(unsafe { V::guard_owned(cursor.into_guard(), value) }))
                        }
                        None => {
                            validate!(old.is_null());
                            Ok(None)
                        }
                    };
                }
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
    }

    pub fn all(&mut self) -> PrefixGuard<'g, '_, K, V> {
        let cursor = cursor::Prefix::<K::Read<'_>, (), _, cursor::path::Discard>::new_root(
            &mut self.smr,
            self.raw.root(),
        );

        PrefixGuard {
            root: cursor.edge(),
            key: K::Write::from(K::Read::default()),
            guard: cursor.into_guard().guard_prefix(),
        }
    }

    pub fn prefix<'k>(
        &mut self,
        prefix: impl Into<K::Read<'k>>,
    ) -> Option<PrefixGuard<'g, '_, K, V>> {
        let prefix = prefix.into();

        let cursor = cursor::Prefix::<_, (), _, cursor::path::Discard>::new_prefix(
            &mut self.smr,
            self.raw.root(),
            prefix,
        )?;

        Some(PrefixGuard {
            root: cursor.edge(),
            key: K::Write::from(cursor.prefix()),
            guard: cursor.into_guard().guard_prefix(),
        })
    }

    // FIXME: support `Option` for min, max
    pub fn range<'l>(
        &'l mut self,
        min: impl Into<K::Read<'l>>,
        max: impl Into<K::Read<'l>>,
    ) -> Option<RangeGuard<'g, 'l, K, V>> {
        let min = min.into();
        let max = max.into();
        let prefix = self.prefix(min.prefix(&max))?;
        Some(RangeGuard {
            prefix,
            min: K::reborrow(min),
            max: K::reborrow(max),
        })
    }

    pub fn prefix_hybrid<'l, S: Sort>(
        &'l mut self,
        buffer: &'l mut Vec<(K::Write, u64)>,
        limit: usize,
        prefix: impl Into<K::Read<'l>>,
    ) -> Option<LinearizableGuard<'g, 'l, K, V>> {
        let prefix = prefix.into();
        let cursor = cursor::Prefix::<_, (), _, cursor::path::Hybrid<_, _>>::new_prefix(
            &mut self.smr,
            self.raw.root(),
            prefix,
        )?;
        Self::scan_hybrid::<iter::Prefix, S>(buffer, cursor, &(), limit)
    }

    pub fn prefix_pessimistic<'l, S: Sort>(
        &'l mut self,
        buffer: &'l mut Vec<(K::Write, u64)>,
        prefix: impl Into<K::Read<'l>>,
    ) -> Option<LinearizableGuard<'g, 'l, K, V>> {
        let prefix = prefix.into();
        let cursor = cursor::Prefix::<_, (), _, cursor::path::Hybrid<_, _>>::new_prefix(
            &mut self.smr,
            self.raw.root(),
            prefix,
        )?;
        Self::scan_pessimistic::<iter::Prefix, S>(buffer, cursor, &())
    }

    pub fn range_hybrid<'l, S: Sort>(
        &'l mut self,
        buffer: &'l mut Vec<(K::Write, u64)>,
        limit: usize,
        min: impl Into<K::Read<'l>>,
        max: impl Into<K::Read<'l>>,
    ) -> Option<LinearizableGuard<'g, 'l, K, V>> {
        let min = min.into();
        let max = max.into();
        let cursor = cursor::Prefix::<_, (), _, cursor::path::Hybrid<_, _>>::new_range(
            &mut self.smr,
            self.raw.root(),
            min,
            max,
        )?;
        Self::scan_hybrid::<iter::Range, S>(buffer, cursor, &(min, max), limit)
    }

    pub fn range_pessimistic<'l, S: Sort>(
        &'l mut self,
        buffer: &'l mut Vec<(K::Write, u64)>,
        min: impl Into<K::Read<'l>>,
        max: impl Into<K::Read<'l>>,
    ) -> Option<LinearizableGuard<'g, 'l, K, V>> {
        let min = min.into();
        let max = max.into();
        let cursor = cursor::Prefix::<_, (), _, cursor::path::Hybrid<_, _>>::new_range(
            &mut self.smr,
            self.raw.root(),
            min,
            max,
        )?;
        Self::scan_pessimistic::<iter::Range, S>(buffer, cursor, &(min, max))
    }

    fn scan_hybrid<'l, S, O>(
        buffer: &'l mut Vec<(K::Write, u64)>,
        cursor: cursor::Prefix<
            'g,
            'l,
            K::Read<'l>,
            (),
            V,
            cursor::path::Hybrid<'g, K::Read<'l>, ()>,
        >,
        arg: &S::Input<'l, K>,
        limit: usize,
    ) -> Option<LinearizableGuard<'g, 'l, K, V>>
    where
        S: Scan,
        O: Sort,
    {
        match Self::scan_optimistic::<S, O>(buffer, &cursor, arg, limit) {
            Ok(()) => Some(LinearizableGuard {
                guard: unsafe { V::downgrade_guard(cursor.into_guard()) },
                buffer,
            }),
            Err(()) => Self::scan_pessimistic::<S, O>(buffer, cursor, arg),
        }
    }

    fn scan_optimistic<'l, S, O>(
        buffer: &mut Vec<(K::Write, u64)>,
        cursor: &cursor::Prefix<
            'g,
            'l,
            K::Read<'l>,
            (),
            V,
            cursor::path::Hybrid<'g, K::Read<'l>, ()>,
        >,
        arg: &S::Input<'l, K>,
        limit: usize,
    ) -> Result<(), ()>
    where
        S: Scan,
        O: Sort,
    {
        S::scan::<_, _, _, O, _>(cursor, arg, |key, value| buffer.push((key.clone(), value)));

        for retry in 0..=limit {
            let mut dirty = false;
            let mut len = 0;

            S::scan::<_, _, _, O, _>(cursor, arg, |new_key, new_value| {
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

    fn scan_pessimistic<'l, S, O>(
        buffer: &'l mut Vec<(K::Write, u64)>,
        mut cursor: cursor::Prefix<
            'g,
            'l,
            K::Read<'l>,
            (),
            V,
            cursor::path::Hybrid<'g, K::Read<'l>, ()>,
        >,
        arg: &S::Input<'l, K>,
    ) -> Option<LinearizableGuard<'g, 'l, K, V>>
    where
        S: Scan,
        O: Sort,
    {
        Self::lock_prefix(&mut cursor)?;
        S::scan::<_, _, _, O, _>(&cursor, arg, |key, value| buffer.push((key.clone(), value)));
        Self::unlock_prefix(&mut cursor);

        Some(LinearizableGuard {
            guard: unsafe { V::downgrade_guard(cursor.into_guard()) },
            buffer,
        })
    }

    fn lock_prefix<'k>(
        cursor: &mut cursor::Prefix<
            'g,
            '_,
            K::Read<'k>,
            (),
            V,
            cursor::path::Hybrid<'g, K::Read<'k>, ()>,
        >,
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
        cursor: &mut cursor::Prefix<
            'g,
            '_,
            K::Read<'k>,
            (),
            V,
            cursor::path::Hybrid<'g, K::Read<'k>, ()>,
        >,
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

pub struct PrefixGuard<'g, 'l, K: Key, V: Value> {
    guard: hazard::PrefixGuard<'g, 'l, V>,
    root: &'g Atomic128<Edge<()>>,
    key: K::Write,
}

impl<'g, 'l, K, V> PrefixGuard<'g, 'l, K, V>
where
    K: Key,
    V: Value,
{
    #[inline]
    pub fn iter<S: Sort>(&self) -> PrefixIter<'g, '_, K, V, S> {
        PrefixIter {
            guard: &self.guard,
            iter: unsafe {
                crate::raw::iter::PrefixIter::new_unchecked(self.root, self.key.clone())
            },
        }
    }

    #[inline]
    pub fn values<S: Sort>(&self) -> PrefixValueIter<'g, '_, V, S> {
        PrefixValueIter {
            guard: &self.guard,
            iter: unsafe { crate::raw::iter::PrefixIter::new_unchecked(self.root, key::Ignore) },
        }
    }
}

pub struct PrefixValueIter<'g, 'l, V: Value, S: crate::iter::Sort> {
    guard: &'l hazard::PrefixGuard<'g, 'l, V>,
    iter: crate::raw::iter::PrefixIter<'g, 'l, key::Ignore, (), S>,
}

impl<'g, 'l, V, S> PrefixValueIter<'g, 'l, V, S>
where
    V: Value,
    S: crate::iter::Sort,
{
    #[inline]
    pub fn for_each<F: FnMut(V::Borrow<'l>)>(self, mut apply: F) {
        self.iter
            .for_each(|key::Ignore, value| apply(unsafe { V::guard_borrow(self.guard, value) }))
    }
}

impl<'g, 'l, V, S> Iterator for PrefixValueIter<'g, 'l, V, S>
where
    V: Value,
    S: crate::iter::Sort,
{
    type Item = V::Borrow<'l>;
    fn next(&mut self) -> Option<Self::Item> {
        self.iter
            .lend()
            .map(|(key::Ignore, value)| unsafe { V::guard_borrow(self.guard, value) })
    }
}

pub struct PrefixIter<'g, 'l, K: Key, V: Value, S: crate::iter::Sort> {
    guard: &'l hazard::PrefixGuard<'g, 'l, V>,
    iter: crate::raw::iter::PrefixIter<'g, 'l, K::Write, (), S>,
}

impl<'g, 'l, K, V, S> PrefixIter<'g, 'l, K, V, S>
where
    K: Key,
    V: Value,
    S: crate::iter::Sort,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(K::Borrow<'_>, V::Borrow<'l>)> {
        self.iter.lend().map(|(key, value)| {
            (unsafe { K::borrow_writer_unchecked(key) }, unsafe {
                V::guard_borrow(self.guard, value)
            })
        })
    }

    #[inline]
    pub fn for_each<F: FnMut(K::Borrow<'_>, V::Borrow<'l>)>(self, mut apply: F) {
        self.iter.for_each(|key, value| {
            apply(unsafe { K::borrow_writer_unchecked(key) }, unsafe {
                V::guard_borrow(self.guard, value)
            })
        })
    }
}

impl<'g, 'l, K, V, S> Iterator for PrefixIter<'g, 'l, K, V, S>
where
    K: Key,
    V: Value,
    S: crate::iter::Sort,
{
    type Item = (K, V::Borrow<'l>);
    fn next(&mut self) -> Option<Self::Item> {
        self.iter.lend().map(|(key, value)| {
            (unsafe { K::from_writer_unchecked(key.clone()) }, unsafe {
                V::guard_borrow(self.guard, value)
            })
        })
    }
}

pub struct RangeGuard<'g, 'l, K: Key, V: Value> {
    prefix: PrefixGuard<'g, 'l, K, V>,
    min: K::Read<'l>,
    max: K::Read<'l>,
}

impl<'g, 'l, K, V> RangeGuard<'g, 'l, K, V>
where
    K: Key,
    V: Value,
{
    #[inline]
    pub fn iter<S: Sort>(&self) -> RangeIter<'g, '_, K, V, S> {
        RangeIter {
            guard: &self.prefix.guard,
            iter: unsafe {
                crate::raw::iter::RangeIter::new_unchecked(
                    self.prefix.root,
                    self.prefix.key.clone(),
                    K::reborrow(self.min),
                    K::reborrow(self.max),
                )
            },
        }
    }
}

pub struct RangeIter<'g, 'l, K: Key, V: Value, S: Sort> {
    guard: &'l hazard::PrefixGuard<'g, 'l, V>,
    iter: crate::raw::iter::RangeIter<'g, 'l, K, (), S>,
}

impl<'g, 'l, K, V, S> RangeIter<'g, 'l, K, V, S>
where
    K: Key,
    V: Value,
    S: Sort,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(K::Borrow<'_>, V::Borrow<'l>)> {
        self.iter.lend().map(|(key, value)| {
            (unsafe { K::borrow_writer_unchecked(key) }, unsafe {
                V::guard_borrow(self.guard, value)
            })
        })
    }

    #[inline]
    pub fn for_each<F: FnMut(K::Borrow<'_>, V::Borrow<'l>)>(self, mut apply: F) {
        self.iter.for_each(|key, value| {
            apply(unsafe { K::borrow_writer_unchecked(key) }, unsafe {
                V::guard_borrow(self.guard, value)
            })
        })
    }
}

impl<'g, 'l, K, V, S> Iterator for RangeIter<'g, 'l, K, V, S>
where
    K: Key,
    V: Value,
    S: Sort,
{
    type Item = (K, V::Borrow<'l>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.iter.lend().map(|(key, value)| {
            (unsafe { K::from_writer_unchecked(key.clone()) }, unsafe {
                V::guard_borrow(self.guard, value)
            })
        })
    }
}
