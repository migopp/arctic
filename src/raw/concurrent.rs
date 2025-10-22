use core::sync::atomic::Ordering;

use polonius_the_crab::polonius;
use polonius_the_crab::polonius_return;
use ribbit::atomic::Atomic128;

use crate::byte;
use crate::edge;
use crate::key;
use crate::key::Read as _;
use crate::node;
use crate::raw::cursor;
use crate::raw::iter;
use crate::raw::sequential;
use crate::raw::Cursor;
use crate::raw::Op;
use crate::smr;
use crate::stat;
use crate::Edge;
use crate::Key;
use crate::Value;

pub(crate) struct Map<V> {
    smr: smr::Global<V>,
    raw: sequential::Map<V>,
}

unsafe impl<V: Send + Sync> Sync for Map<V> {}

impl<V> Default for Map<V> {
    fn default() -> Self {
        Self {
            smr: smr::Global::default(),
            raw: sequential::Map::<V>::default(),
        }
    }
}

impl<V> Map<V> {
    #[inline]
    pub(crate) fn pin(&self) -> MapRef<V> {
        MapRef {
            smr: self.smr.pin(),
            raw: &self.raw,
        }
    }

    #[inline]
    pub(crate) fn as_sequential(&mut self) -> &mut sequential::Map<V> {
        &mut self.raw
    }
}

pub(crate) struct MapRef<'g, V> {
    smr: smr::Local<'g, V>,
    raw: &'g sequential::Map<V>,
}

impl<'g, V> MapRef<'g, V>
where
    V: Value + Send + Sync,
{
    #[inline]
    pub(crate) fn get<R: key::Read>(&mut self, key: R) -> Option<V::Shared<'g, '_>> {
        Cursor::new(&mut self.smr, self.raw.root(), key).traverse_value()
    }

    #[inline]
    pub(crate) fn update<R: key::Read>(&mut self, key: R, value: V) -> Option<V::Owned<'g, '_>> {
        let leaf = Edge::new_leaf(byte::Array::EMPTY, value);
        unsafe { self.compare_exchange(key, |old| old.with_data(leaf.data())) }
    }

    #[inline]
    pub(crate) fn remove<R: key::Read>(&mut self, key: R) -> Option<V::Owned<'g, '_>> {
        unsafe { self.compare_exchange(key, |_| Edge::DEFAULT) }
    }

    #[inline]
    unsafe fn compare_exchange<R, F>(&mut self, key: R, mut exchange: F) -> Option<V::Owned<'g, '_>>
    where
        R: key::Read,
        F: FnMut(ribbit::Packed<Edge<V>>) -> ribbit::Packed<Edge<V>>,
    {
        let mut map = self;

        // Cursed workaround for:
        // https://github.com/rust-lang/rust/issues/54663
        polonius!(|map| -> Option<V::Owned<'g, 'polonius>> {
            if let Ok(old) = map.compare_exchange_optimistic::<R, _>(key, &mut exchange) {
                polonius_return!(old);
            }
        });

        map.compare_exchange_pessimistic(key, exchange)
    }

    #[inline]
    unsafe fn compare_exchange_optimistic<R, F>(
        &mut self,
        key: R,
        exchange: F,
    ) -> Result<Option<V::Owned<'g, '_>>, ()>
    where
        R: key::Read,
        F: FnMut(ribbit::Packed<Edge<V>>) -> ribbit::Packed<Edge<V>>,
    {
        self.compare_exchange_impl::<_, cursor::Optimistic, _>(key, exchange)
    }

    #[cold]
    unsafe fn compare_exchange_pessimistic<R, F>(
        &mut self,
        key: R,
        exchange: F,
    ) -> Option<V::Owned<'g, '_>>
    where
        R: key::Read,
        F: FnMut(ribbit::Packed<Edge<V>>) -> ribbit::Packed<Edge<V>>,
    {
        self.compare_exchange_impl::<_, cursor::Pessimistic<R, V>, _>(key, exchange)
            .unwrap()
    }

    /// # SAFETY
    ///
    /// Caller must guarantee that `exchange` removes the old value from the tree,
    /// or else we will duplicate ownership.
    #[inline]
    unsafe fn compare_exchange_impl<R, H, F>(
        &mut self,
        key: R,
        mut exchange: F,
    ) -> Result<Option<V::Owned<'g, '_>>, H::PopError>
    where
        R: key::Read,
        H: cursor::History<'g, R, V>,
        F: FnMut(ribbit::Packed<Edge<V>>) -> ribbit::Packed<Edge<V>>,
    {
        let mut cursor = Cursor::<R, V, H>::new(&mut self.smr, self.raw.root(), key);

        loop {
            let old = match cursor.traverse_exact() {
                None => return Ok(None),
                Some(Ok(old)) => old,
                Some(Err(())) => {
                    Self::freeze(&mut cursor, key)?;
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
                        V::new_owned(cursor.into_guard(), old.data().into_leaf())
                    }))
                } else {
                    validate!(old.is_null());
                    Ok(None)
                };
            }
        }
    }

    #[inline]
    pub(crate) fn insert<R: key::Read>(&mut self, key: R, value: V) -> Option<V::Owned<'g, '_>> {
        let leaf = Edge::new_leaf(byte::Array::EMPTY, value);
        let mut map = &mut *self;

        // Cursed workaround for:
        // https://github.com/rust-lang/rust/issues/54663
        polonius!(|map| -> Option<V::Owned<'g, 'polonius>> {
            if let Ok(old) = map.insert_optimistic(key, leaf) {
                polonius_return!(old);
            }
        });

        map.insert_pessimistic(key, leaf)
    }

    #[inline]
    fn insert_optimistic<R: key::Read>(
        &mut self,
        key: R,
        leaf: ribbit::Packed<Edge<V>>,
    ) -> Result<Option<V::Owned<'g, '_>>, ()> {
        self.insert_impl::<_, cursor::Optimistic>(key, leaf)
    }

    #[cold]
    fn insert_pessimistic<R: key::Read>(
        &mut self,
        key: R,
        leaf: ribbit::Packed<Edge<V>>,
    ) -> Option<V::Owned<'g, '_>> {
        stat::increment(stat::Counter::InsertPessimistic);
        self.insert_impl::<_, cursor::Pessimistic<R, V>>(key, leaf)
            .unwrap()
    }

    #[inline]
    fn insert_impl<R: key::Read, H: cursor::History<'g, R, V>>(
        &mut self,
        key: R,
        leaf: ribbit::Packed<Edge<V>>,
    ) -> Result<Option<V::Owned<'g, '_>>, H::PopError> {
        let mut cursor = Cursor::<R, V, H>::new(&mut self.smr, self.raw.root(), key);

        loop {
            let (op, old, new) = match cursor.traverse_or_insert(leaf) {
                Ok(cas) => cas,
                Err(()) => {
                    Self::freeze(&mut cursor, key)?;
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
                            V::new_owned(cursor.into_guard(), old.data().into_leaf())
                        }));
                    } else {
                        validate!(old.is_null());
                        return Ok(None);
                    }
                }
                Ok(_) => {
                    stat::increment(op);
                    unsafe { Self::retire(&mut cursor, op, key, old) };
                }
                Err(_) => {
                    // Does not go through SMR because `new` is still thread-local
                    unsafe { Self::deallocate(op, new) };
                }
            }
        }
    }

    #[cold]
    fn freeze<R: key::Read, H: cursor::History<'g, R, V>>(
        cursor: &mut Cursor<'g, '_, R, V, H>,
        key: R,
    ) -> Result<(), H::PopError> {
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
                    .deallocate_unchecked(stat::Counter::FreeConflict)
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

    pub(crate) fn prefix_non_linearizable<'l, R, W, S>(
        &'l mut self,
        prefix: R,
    ) -> PrefixNonLinearizable<'g, 'l, W, V, S>
    where
        R: key::Read,
        W: key::Write + From<R>,
        S: crate::iter::Sort,
    {
        let mut cursor =
            Cursor::<R, V, cursor::Optimistic>::new(&mut self.smr, self.raw.root(), prefix);

        let iter = match cursor.traverse_prefix() {
            Some(_) => unsafe {
                iter::LeafIter::new(cursor.root(), W::from(prefix.slice(cursor.bit())))
            },
            None => iter::LeafIter::empty(),
        };

        PrefixNonLinearizable {
            iter,
            _guard: cursor.into_guard(),
        }
    }

    pub(crate) fn range_non_linearizable<'l, R, W>(
        &'l mut self,
        min: R,
        max: R,
    ) -> RangeIter<'g, 'l, R, W, V>
    where
        R: key::Read,
        W: key::Write<Len = usize> + PartialOrd<R> + From<R>,
    {
        let prefix = min.prefix(&max);
        let mut cursor =
            Cursor::<_, _, cursor::Optimistic>::new(&mut self.smr, self.raw.root(), prefix);

        let iter = match cursor.traverse_prefix() {
            // FIXME: do not need to hold SMR guard if iterator is empty
            None => iter::RangeIter::<R, W, V>::empty(),
            Some(_) => unsafe {
                iter::RangeIter::<R, W, V>::new(
                    cursor.root(),
                    W::from(prefix.slice(cursor.bit())),
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

    pub(crate) fn range_pessimistic<'l, K: Key>(
        &'l mut self,
        min: K::Read<'l>,
        max: K::Read<'l>,
        output: &mut Vec<(K, V)>,
    ) {
        let prefix = min.prefix(&max);

        let mut cursor =
            Cursor::<_, _, cursor::Optimistic>::new(&mut self.smr, self.raw.root(), prefix);

        if cursor.traverse_prefix().is_none() {
            return;
        }

        Self::pessimistic(self.raw.root(), cursor, prefix, min, max, output);
    }

    fn pessimistic<'l, K: Key>(
        root: &'g Atomic128<Edge<V>>,
        cursor: Cursor<'g, 'l, K::Read<'l>, V, cursor::Optimistic>,
        prefix: K::Read<'l>,
        min: K::Read<'l>,
        max: K::Read<'l>,
        output: &mut Vec<(K, V)>,
    ) {
        match Self::pessimistic_impl(root, cursor, prefix, min, max, output) {
            Ok(()) => (),
            Err(cursor) => Self::pessimistic_pessimistic(root, cursor, prefix, min, max, output),
        }
    }

    #[cold]
    fn pessimistic_pessimistic<'l, K: Key>(
        root: &'g Atomic128<Edge<V>>,
        cursor: Cursor<'g, 'l, K::Read<'l>, V, cursor::Optimistic>,
        prefix: K::Read<'l>,
        min: K::Read<'l>,
        max: K::Read<'l>,
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

    fn pessimistic_impl<'l, K: Key, H: cursor::History<'g, K::Read<'l>, V>>(
        root: &'g Atomic128<Edge<V>>,
        mut cursor: Cursor<'g, 'l, K::Read<'l>, V, H>,
        prefix: K::Read<'l>,
        min: K::Read<'l>,
        max: K::Read<'l>,
        output: &mut Vec<(K, V)>,
    ) -> Result<(), Cursor<'g, 'l, K::Read<'l>, V, H>> {
        match Self::lock(&mut cursor, prefix) {
            Ok(()) => (),
            Err(_) => return Err(cursor),
        }

        unsafe {
            iter::RangeIter::<K::Read<'l>, K::Write, V>::new(
                cursor.root(),
                K::Write::from(prefix.slice(cursor.bit())),
                min,
                max,
            )
        }
        .for_each(|key, value| output.push((K::from(K::Borrow::from(key)), V::from_u64(value))));

        match Self::unlock(&mut cursor, prefix) {
            Ok(()) => (),
            Err(_) => Self::unlock_pessimistic(root, cursor, prefix),
        }

        Ok(())
    }

    fn lock<'l, R: key::Read, H: cursor::History<'g, R, V>>(
        cursor: &mut Cursor<'g, 'l, R, V, H>,
        prefix: R,
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
    fn unlock_pessimistic<'l, R: key::Read, H: cursor::History<'g, R, V>>(
        root: &'g Atomic128<Edge<V>>,
        cursor: Cursor<'g, 'l, R, V, H>,
        prefix: R,
    ) {
        stat::increment(stat::Counter::UnlockFrozen);

        let mut cursor = cursor.upgrade(root, prefix);

        let Some(_) = cursor.traverse_prefix() else {
            unreachable!("Scan lock must exist");
        };

        Self::unlock(&mut cursor, prefix).unwrap()
    }

    #[inline]
    fn unlock<'l, R: key::Read, H: cursor::History<'g, R, V>>(
        cursor: &mut Cursor<'g, 'l, R, V, H>,
        prefix: R,
    ) -> Result<(), H::PopError> {
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

    pub(crate) fn range_optimistic<'l, K: Key>(
        &'l mut self,
        min: K::Read<'l>,
        max: K::Read<'l>,
        retry: usize,
        output: &mut Vec<(K, V)>,
    ) {
        // FIXME: deduplicate prefix traversal?
        let prefix = min.prefix(&max);

        let mut cursor = Cursor::<K::Read<'l>, V, cursor::Optimistic>::new(
            &mut self.smr,
            self.raw.root(),
            prefix,
        );

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
        .for_each(|key, value| output.push((K::from(K::Borrow::from(key)), V::from_u64(value))));

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
                let new_value = V::from_u64(new_value);

                let old = match output
                    .get_mut(index)
                    .map(|(key, value)| (key.borrow(), value))
                {
                    // Fast path: no change
                    Some((old_borrow, old_value))
                        if old_borrow == new_borrow && *old_value == new_value =>
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

pub(crate) struct RangeIter<'g, 'l, R: key::Read, W, V> {
    iter: iter::RangeIter<'g, R, W, V>,
    _guard: smr::PathGuard<'g, 'l, V>,
}

impl<'g, 'l, R, W, V> RangeIter<'g, 'l, R, W, V>
where
    R: key::Read,
    W: key::Write<Len = usize> + PartialOrd<R>,
{
    #[inline]
    pub(crate) fn lend(&mut self) -> Option<(&W, u64)> {
        self.iter.lend()
    }

    #[inline]
    pub(crate) fn for_each<F: FnMut(&W, u64)>(&mut self, apply: F) {
        self.iter.for_each(apply)
    }
}

pub(crate) struct PrefixNonLinearizable<'g, 'l, W: key::Write, V, S: crate::iter::Sort> {
    iter: iter::LeafIter<'g, W, V, S>,
    _guard: smr::PathGuard<'g, 'l, V>,
}

impl<'g, 'l, W, V, S> PrefixNonLinearizable<'g, 'l, W, V, S>
where
    W: key::Write,
    S: crate::iter::Sort,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(&W, u64)> {
        self.iter.lend()
    }
}
