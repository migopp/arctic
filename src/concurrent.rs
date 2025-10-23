use core::sync::atomic::Ordering;

use polonius_the_crab::polonius;
use polonius_the_crab::polonius_return;
use ribbit::atomic::Atomic128;

use crate::byte;
use crate::cursor;
use crate::edge;
use crate::iter;
use crate::iter::Sort;
use crate::key;
use crate::key::Read as _;
use crate::node;
use crate::sequential;
use crate::smr;
use crate::stat;
use crate::value::Owned;
use crate::value::Shared;
use crate::Cursor;
use crate::Edge;
use crate::Key;
use crate::Op;
use crate::Value;

pub struct Map<K, V: Value> {
    smr: smr::Global<V>,
    raw: sequential::Map<K, V>,
}

unsafe impl<K, V: Value + Send + Sync> Sync for Map<K, V> {}

impl<K, V: Value> Default for Map<K, V> {
    fn default() -> Self {
        Self {
            smr: smr::Global::default(),
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
    smr: smr::Local<'g, V>,
    raw: &'g sequential::Map<K, V>,
}

impl<'g, K, V> MapRef<'g, K, V>
where
    K: Key,
    V: Value + Send + Sync,
{
    #[inline]
    pub fn get(&mut self, key: K::Borrow<'_>) -> Option<Shared<'g, '_, V>> {
        Cursor::new(&mut self.smr, self.raw.root(), K::Read::from(key)).traverse_value()
    }

    #[inline]
    pub fn update(&mut self, key: K::Borrow<'_>, value: V) -> Option<Owned<'g, '_, V>> {
        let leaf = Edge::new_leaf(byte::Array::EMPTY, value);
        unsafe { self.compare_exchange(key, |old| old.with_data(leaf.data())) }
    }

    #[inline]
    pub fn remove(&mut self, key: K::Borrow<'_>) -> Option<Owned<'g, '_, V>> {
        unsafe { self.compare_exchange(key, |_| Edge::DEFAULT) }
    }

    #[inline]
    unsafe fn compare_exchange<F>(
        &mut self,
        key: K::Borrow<'_>,
        mut exchange: F,
    ) -> Option<Owned<'g, '_, V>>
    where
        F: FnMut(ribbit::Packed<Edge<V>>) -> ribbit::Packed<Edge<V>>,
    {
        let mut map = self;

        // Cursed workaround for:
        // https://github.com/rust-lang/rust/issues/54663
        polonius!(|map| -> Option<Owned<'g, 'polonius, V>> {
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
    ) -> Result<Option<Owned<'g, '_, V>>, ()>
    where
        F: FnMut(ribbit::Packed<Edge<V>>) -> ribbit::Packed<Edge<V>>,
    {
        self.compare_exchange_impl::<cursor::Optimistic, _>(key, exchange)
    }

    #[cold]
    unsafe fn compare_exchange_pessimistic<F>(
        &mut self,
        key: K::Borrow<'_>,
        exchange: F,
    ) -> Option<Owned<'g, '_, V>>
    where
        F: FnMut(ribbit::Packed<Edge<V>>) -> ribbit::Packed<Edge<V>>,
    {
        self.compare_exchange_impl::<cursor::Pessimistic<_, _>, _>(key, exchange)
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
    ) -> Result<Option<Owned<'g, '_, V>>, H::PopError>
    where
        H: cursor::History<'g, K::Read<'k>, V>,
        F: FnMut(ribbit::Packed<Edge<V>>) -> ribbit::Packed<Edge<V>>,
    {
        let reader = K::Read::from(key);
        let mut cursor = Cursor::<_, _, H>::new(&mut self.smr, self.raw.root(), reader);

        loop {
            let old = match cursor.traverse_exact() {
                None => return Ok(None),
                Some(Ok(old)) => old,
                Some(Err(())) => {
                    Self::freeze(&mut cursor, reader)?;
                    continue;
                }
            };

            if cursor
                .root()
                .compare_exchange_packed(old, exchange(old), Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return if old.meta().leaf() {
                    Ok(Some(unsafe {
                        V::guard(cursor.into_guard(), old.data().into_leaf())
                    }))
                } else {
                    validate!(old.is_null());
                    Ok(None)
                };
            }
        }
    }

    #[inline]
    pub fn insert(&mut self, key: K::Borrow<'_>, value: V) -> Option<Owned<'g, '_, V>> {
        let leaf = Edge::new_leaf(byte::Array::EMPTY, value);
        let mut map = &mut *self;

        // Cursed workaround for:
        // https://github.com/rust-lang/rust/issues/54663
        polonius!(|map| -> Option<Owned<'g, 'polonius, V>> {
            if let Ok(old) = map.insert_optimistic(key, leaf) {
                polonius_return!(old);
            }
        });

        map.insert_pessimistic(key, leaf)
    }

    #[inline]
    fn insert_optimistic(
        &mut self,
        key: K::Borrow<'_>,
        leaf: ribbit::Packed<Edge<V>>,
    ) -> Result<Option<Owned<'g, '_, V>>, ()> {
        self.insert_impl::<cursor::Optimistic>(key, leaf)
    }

    #[cold]
    fn insert_pessimistic(
        &mut self,
        key: K::Borrow<'_>,
        leaf: ribbit::Packed<Edge<V>>,
    ) -> Option<Owned<'g, '_, V>> {
        stat::increment(stat::Counter::InsertPessimistic);
        self.insert_impl::<cursor::Pessimistic<_, _>>(key, leaf)
            .unwrap()
    }

    #[inline]
    fn insert_impl<'k, H>(
        &mut self,
        key: K::Borrow<'k>,
        leaf: ribbit::Packed<Edge<V>>,
    ) -> Result<Option<Owned<'g, '_, V>>, H::PopError>
    where
        H: cursor::History<'g, K::Read<'k>, V>,
    {
        let reader = K::Read::from(key);
        let mut cursor = Cursor::<_, _, H>::new(&mut self.smr, self.raw.root(), reader);

        loop {
            let (op, old, new) = match cursor.traverse_or_insert(leaf) {
                Ok(cas) => cas,
                Err(()) => {
                    Self::freeze(&mut cursor, reader)?;
                    continue;
                }
            };

            validate!(!old.meta().frozen());

            match cursor.root().compare_exchange_packed(
                old,
                new,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) if op == Op::Edge(edge::Op::Insert) => {
                    stat::increment(op);
                    if old.meta().leaf() {
                        return Ok(Some(unsafe {
                            V::guard(cursor.into_guard(), old.data().into_leaf())
                        }));
                    } else {
                        validate!(old.is_null());
                        return Ok(None);
                    }
                }
                Ok(_) => {
                    stat::increment(op);
                    unsafe { Self::retire(&mut cursor, op, reader, old) };
                }
                Err(_) => {
                    // Does not go through SMR because `new` is still thread-local
                    unsafe { Self::deallocate(op, new) };
                }
            }
        }
    }

    #[cold]
    fn freeze<'k, H>(
        cursor: &mut Cursor<'g, '_, K::Read<'k>, V, H>,
        key: K::Read<'k>,
    ) -> Result<(), H::PopError>
    where
        H: cursor::History<'g, K::Read<'k>, V>,
    {
        let mut node = cursor.pop()?;
        let mut edge = cursor.root().load_packed(Ordering::Acquire);

        loop {
            while edge.meta().frozen() {
                node = cursor.pop()?;
                edge = cursor.root().load_packed(Ordering::Acquire);
            }

            let meta = edge.meta();
            let data = edge.data();

            // Should be impossible to freeze leaf
            validate!(!meta.leaf());

            // Already helped by another thread
            if !data.is_ref(node) {
                return Ok(());
            }

            let (op, new) = node.replace(meta);

            match cursor.root().compare_exchange_packed(
                edge,
                new.with_data(new.data().with_scan(data.scan())),
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => unsafe {
                    stat::increment(op);
                    Self::retire(cursor, Op::Node(op), key, edge);
                    return Ok(());
                },
                Err(conflict) => unsafe {
                    Self::deallocate(Op::Node(op), new);
                    edge = conflict;
                },
            };
        }
    }

    #[cold]
    unsafe fn deallocate(op: Op, edge: ribbit::Packed<Edge<V>>) {
        match op {
            Op::Node(node::Op::Destroy | node::Op::Compress)
            | Op::Edge(edge::Op::Insert | edge::Op::Remove) => (),
            Op::Node(node::Op::Grow | node::Op::Replace | node::Op::Shrink)
            | Op::Edge(edge::Op::Create | edge::Op::Expand) => unsafe {
                validate!(edge.is_node());
                edge.data()
                    .deallocate_node_unchecked(stat::Counter::FreeConflict)
            },
        }
    }

    #[cold]
    unsafe fn retire<R: key::Read, H: cursor::History<'g, R, V>>(
        cursor: &mut Cursor<'g, '_, R, V, H>,
        op: Op,
        key: R,
        edge: ribbit::Packed<Edge<V>>,
    ) {
        match op {
            Op::Edge(_) => return,
            Op::Node(_) => (),
        }

        validate!(edge.is_node());

        let prefix = key.peek(byte::Len::MAX.min_bits(cursor.bit()));

        unsafe {
            cursor.retire(edge.with_meta(edge.meta().with_key(prefix)));
        }
    }

    pub fn prefix<'k>(
        &mut self,
        prefix: impl Into<K::Read<'k>>,
    ) -> Option<PrefixGuard<'g, '_, K, V>> {
        let prefix = prefix.into();

        let mut cursor =
            Cursor::<_, _, cursor::Optimistic>::new(&mut self.smr, self.raw.root(), prefix);

        cursor.traverse_prefix()?;

        Some(PrefixGuard {
            root: cursor.root(),
            key: K::Write::from(prefix.slice(cursor.bit())),
            guard: cursor.into_guard(),
        })
    }

    pub fn range_non_linearizable<'l, 'k>(
        &'l mut self,
        min: impl Into<K::Read<'k>>,
        max: impl Into<K::Read<'k>>,
    ) -> RangeIter<'g, 'l, 'k, K, V> {
        let min = min.into();
        let max = max.into();
        let prefix = min.prefix(&max);

        let mut cursor =
            Cursor::<_, _, cursor::Optimistic>::new(&mut self.smr, self.raw.root(), prefix);

        let iter = match cursor.traverse_prefix() {
            // FIXME: do not need to hold SMR guard if iterator is empty
            None => iter::RangeIter::empty(),
            Some(_) => unsafe {
                iter::RangeIter::new(
                    cursor.root(),
                    K::Write::from(prefix.slice(cursor.bit())),
                    min,
                    max,
                )
            },
        };

        RangeIter {
            iter,
            _guard: cursor.into_guard(),
        }
    }

    pub fn range_pessimistic<'l>(
        &'l mut self,
        min: impl Into<K::Read<'l>>,
        max: impl Into<K::Read<'l>>,
        output: &mut Vec<(K, V)>,
    ) {
        let min = min.into();
        let max = max.into();
        let prefix = min.prefix(&max);

        let mut cursor =
            Cursor::<_, _, cursor::Optimistic>::new(&mut self.smr, self.raw.root(), prefix);

        if cursor.traverse_prefix().is_none() {
            return;
        }

        Self::pessimistic(self.raw.root(), cursor, prefix, min, max, output);
    }

    fn pessimistic<'l, 'k>(
        root: &'g Atomic128<Edge<V>>,
        cursor: Cursor<'g, 'l, K::Read<'k>, V, cursor::Optimistic>,
        prefix: K::Read<'k>,
        min: K::Read<'k>,
        max: K::Read<'k>,
        output: &mut Vec<(K, V)>,
    ) {
        match Self::pessimistic_impl(root, cursor, prefix, min, max, output) {
            Ok(()) => (),
            Err(cursor) => Self::pessimistic_pessimistic(root, cursor, prefix, min, max, output),
        }
    }

    #[cold]
    fn pessimistic_pessimistic<'l, 'k>(
        root: &'g Atomic128<Edge<V>>,
        cursor: Cursor<'g, 'l, K::Read<'k>, V, cursor::Optimistic>,
        prefix: K::Read<'k>,
        min: K::Read<'k>,
        max: K::Read<'k>,
        output: &mut Vec<(K, V)>,
    ) {
        stat::increment(stat::Counter::LockFrozen);

        let mut cursor = cursor.upgrade(root, prefix);

        let Some(_) = cursor.traverse_prefix() else {
            return;
        };

        match Self::pessimistic_impl(root, cursor, prefix, min, max, output) {
            Ok(()) => (),
            Err(_) => unreachable!(),
        }
    }

    fn pessimistic_impl<'l, 'k, H: cursor::History<'g, K::Read<'k>, V>>(
        root: &'g Atomic128<Edge<V>>,
        mut cursor: Cursor<'g, 'l, K::Read<'k>, V, H>,
        prefix: K::Read<'k>,
        min: K::Read<'k>,
        max: K::Read<'k>,
        output: &mut Vec<(K, V)>,
    ) -> Result<(), Cursor<'g, 'l, K::Read<'k>, V, H>> {
        match Self::lock(&mut cursor, prefix) {
            Ok(()) => (),
            Err(_) => return Err(cursor),
        }

        unsafe {
            iter::RangeIter::<K::Read<'k>, K::Write, V>::new(
                cursor.root(),
                K::Write::from(prefix.slice(cursor.bit())),
                min,
                max,
            )
        }
        .for_each(|key, value| {
            output.push((K::from(K::Borrow::from(key)), unsafe { V::from_u64(value) }))
        });

        match Self::unlock(&mut cursor, prefix) {
            Ok(()) => (),
            Err(_) => Self::unlock_pessimistic(root, cursor, prefix),
        }

        Ok(())
    }

    fn lock<'k, H: cursor::History<'g, K::Read<'k>, V>>(
        cursor: &mut Cursor<'g, '_, K::Read<'k>, V, H>,
        prefix: K::Read<'k>,
    ) -> Result<(), H::PopError> {
        let mut edge = cursor.root().load_packed(Ordering::Relaxed);

        loop {
            // No need to lock leaf
            if edge.meta().leaf() {
                return Ok(());
            }

            if edge.meta().frozen() || edge.data().scan() {
                match cursor.wait_for_scan(stat::Counter::ScanScan) {
                    Ok(safe) if !edge.meta().frozen() => edge = safe,
                    Ok(_) | Err(()) => {
                        Self::freeze(cursor, prefix)?;
                    }
                }
            }

            match cursor.root().compare_exchange_packed(
                edge,
                edge.with_data(edge.data().with_scan(true)),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Ok(()),
                Err(conflict) => {
                    core::hint::spin_loop();
                    edge = conflict;
                }
            }
        }
    }

    #[cold]
    fn unlock_pessimistic<'k, H: cursor::History<'g, K::Read<'k>, V>>(
        root: &'g Atomic128<Edge<V>>,
        cursor: Cursor<'g, '_, K::Read<'k>, V, H>,
        prefix: K::Read<'k>,
    ) {
        stat::increment(stat::Counter::UnlockFrozen);

        let mut cursor = cursor.upgrade(root, prefix);

        let Some(_) = cursor.traverse_prefix() else {
            unreachable!("Scan lock must exist");
        };

        Self::unlock(&mut cursor, prefix).unwrap()
    }

    #[inline]
    fn unlock<'k, H>(
        cursor: &mut Cursor<'g, '_, K::Read<'k>, V, H>,
        prefix: K::Read<'k>,
    ) -> Result<(), H::PopError>
    where
        H: cursor::History<'g, K::Read<'k>, V>,
    {
        let mut edge = cursor.root().load_packed(Ordering::Relaxed);

        if edge.meta().leaf() {
            return Ok(());
        }

        loop {
            validate!(edge.data().scan());

            if edge.meta().frozen() {
                Self::freeze(cursor, prefix)?;
                edge = cursor
                    .traverse_prefix()
                    .expect("Scan bit must be reachable");
                continue;
            }

            match cursor.root().compare_exchange_packed(
                edge,
                edge.with_data(edge.data().with_scan(false)),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Ok(()),
                Err(conflict) => {
                    core::hint::spin_loop();
                    edge = conflict;
                }
            }
        }
    }

    pub fn range_optimistic<'l, 'k>(
        &'l mut self,
        min: impl Into<K::Read<'k>>,
        max: impl Into<K::Read<'k>>,
        retry: usize,
        output: &mut Vec<(K, V)>,
    ) {
        let min = min.into();
        let max = max.into();
        let prefix = min.prefix(&max);

        let mut cursor =
            Cursor::<_, _, cursor::Optimistic>::new(&mut self.smr, self.raw.root(), prefix);

        let Some(_) = cursor.traverse_prefix() else {
            return;
        };

        let len = output.len();
        unsafe {
            iter::RangeIter::new(
                cursor.root(),
                K::Write::from(prefix.slice(cursor.bit())),
                min,
                max,
            )
        }
        .for_each(|key, value| {
            output.push((K::from(K::Borrow::from(key)), unsafe { V::from_u64(value) }))
        });

        for retry in 0..=retry {
            let mut iter = unsafe {
                iter::RangeIter::new(
                    cursor.root(),
                    K::Write::from(prefix.slice(cursor.bit())),
                    min,
                    max,
                )
            };
            let mut dirty = false;
            let mut len = len;

            iter.for_each(|new_writer, new_value| {
                let index = len;
                len += 1;

                let new_borrow = K::Borrow::from(new_writer);
                let new_value = unsafe { V::from_u64(new_value) };

                let old = match output
                    .get_mut(index)
                    .map(|(key, value)| (key.borrow(), value))
                {
                    // Fast path: no change
                    Some((old_borrow, old_value))
                        if old_borrow == new_borrow =>
                        // FIXME: use physical equality && *old_value == new_value =>
                    {
                        return;
                    }
                    old => old,
                };

                crate::cold();

                dirty = true;

                match old {
                    Some((old_borrow, old_value)) if old_borrow == new_borrow => {
                        *old_value = new_value;
                    }
                    Some((old_borrow, _)) if old_borrow < new_borrow => {
                        let high = output[len..]
                            .iter()
                            .map(|(key, value)| (key.borrow(), value))
                            .position(|(old_borrow, _)| old_borrow >= new_borrow)
                            .map(|offset| len + offset)
                            .unwrap_or(output.len());
                        output.drain(index..high);
                        len = index;
                    }
                    None | Some(_) => {
                        let new_key = K::from(K::Borrow::from(new_writer));
                        output.insert(index, (new_key, new_value));
                    }
                };
            });

            if len == output.len() && !dirty {
                stat::record(stat::Record::RangeConflict, retry as u64);
                return;
            }

            crate::cold();
            validate!(output.len() <= len);
            output.truncate(len);
        }

        Self::pessimistic(self.raw.root(), cursor, prefix, min, max, output);
    }
}

pub struct PrefixGuard<'g, 'l, K: Key, V: Value> {
    guard: smr::PathGuard<'g, 'l, V>,
    root: &'g Atomic128<Edge<V>>,
    key: K::Write,
}

impl<'g, 'l, K, V> PrefixGuard<'g, 'l, K, V>
where
    K: Key,
    V: Value,
{
    #[inline]
    pub fn iter<S: Sort>(&self) -> PrefixIter<'_, 'g, 'l, K, V, S> {
        PrefixIter {
            guard: &self.guard,
            iter: unsafe { iter::LeafIter::new(self.root, self.key.clone()) },
        }
    }
}

pub struct PrefixIter<'guard, 'g, 'l, K: Key, V: Value, S: crate::iter::Sort> {
    guard: &'guard smr::PathGuard<'g, 'l, V>,
    iter: iter::LeafIter<'g, K::Write, V, S>,
}

impl<'guard, 'g, 'l, K, V, S> PrefixIter<'guard, 'g, 'l, K, V, S>
where
    K: Key,
    V: Value,
    S: crate::iter::Sort,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(K::Borrow<'_>, V::Borrow<'guard>)> {
        self.iter.lend().map(|(key, value)| {
            (K::Borrow::from(key), unsafe {
                V::borrow_from_u64(self.guard, value)
            })
        })
    }
}

impl<'guard, 'g, 'l, K, V, S> Iterator for PrefixIter<'guard, 'g, 'l, K, V, S>
where
    K: Key,
    V: Value,
    S: crate::iter::Sort,
{
    type Item = (K, V::Borrow<'guard>);
    fn next(&mut self) -> Option<Self::Item> {
        self.lend().map(|(key, value)| (K::from(key), value))
    }
}

pub struct RangeIter<'g, 'l, 'k, K: Key, V: Value> {
    iter: iter::RangeIter<'g, K::Read<'k>, K::Write, V>,
    _guard: smr::PathGuard<'g, 'l, V>,
}

impl<'g, 'l, 'k, K, V> RangeIter<'g, 'l, 'k, K, V>
where
    K: Key,
    V: Value,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(K::Borrow<'_>, V)> {
        self.iter
            .lend()
            .map(|(key, value)| (K::Borrow::from(key), unsafe { V::from_u64(value) }))
    }

    #[inline]
    pub fn for_each<F: FnMut(K::Borrow<'_>, V)>(&mut self, mut apply: F) {
        self.iter
            .for_each(|key, value| apply(K::Borrow::from(key), unsafe { V::from_u64(value) }))
    }
}

impl<'g, 'l, 'k, K, V> Iterator for RangeIter<'g, 'l, 'k, K, V>
where
    K: Key,
    V: Value,
{
    type Item = (K, V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.lend().map(|(key, value)| (K::from(key), value))
    }
}
