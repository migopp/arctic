use core::sync::atomic::Ordering;

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

#[derive(Default)]
pub(crate) struct Map {
    smr: smr::Global,
    raw: sequential::Map,
}

unsafe impl Sync for Map {}

impl Map {
    #[inline]
    pub(crate) fn pin(&self) -> MapRef {
        MapRef {
            smr: self.smr.pin(),
            raw: &self.raw,
        }
    }

    #[inline]
    pub(crate) fn as_sequential(&mut self) -> &mut sequential::Map {
        &mut self.raw
    }
}

pub(crate) struct MapRef<'g> {
    smr: smr::Local<'g>,
    raw: &'g sequential::Map,
}

impl<'g> MapRef<'g> {
    #[inline]
    pub(crate) fn get<R: key::Read>(&mut self, key: R) -> Option<u64> {
        Cursor::new(&mut self.smr, self.raw.root(), key).traverse_value()
    }

    #[inline]
    pub(crate) fn update<R: key::Read>(&mut self, key: R, value: u64) -> Option<u64> {
        self.compare_exchange(key, |old| Edge::new_leaf(old.meta().key(), value))
    }

    #[inline]
    pub(crate) fn remove<R: key::Read>(&mut self, key: R) -> Option<u64> {
        self.compare_exchange(key, |_| Edge::DEFAULT)
    }

    #[inline]
    fn compare_exchange<R, F>(&mut self, key: R, mut exchange: F) -> Option<u64>
    where
        R: key::Read,
        F: FnMut(ribbit::Packed<Edge>) -> ribbit::Packed<Edge>,
    {
        match self.compare_exchange_optimistic(key, &mut exchange) {
            Ok(old) => old,
            Err(()) => self.compare_exchange_pessimistic(key, exchange),
        }
    }

    #[inline]
    fn compare_exchange_optimistic<R, F>(&mut self, key: R, exchange: F) -> Result<Option<u64>, ()>
    where
        R: key::Read,
        F: FnMut(ribbit::Packed<Edge>) -> ribbit::Packed<Edge>,
    {
        self.compare_exchange_impl::<_, cursor::Optimistic<R>, _>(key, exchange)
    }

    #[cold]
    fn compare_exchange_pessimistic<R, F>(&mut self, key: R, exchange: F) -> Option<u64>
    where
        R: key::Read,
        F: FnMut(ribbit::Packed<Edge>) -> ribbit::Packed<Edge>,
    {
        self.compare_exchange_impl::<_, cursor::Pessimistic<R>, _>(key, exchange)
            .unwrap()
    }

    #[inline]
    fn compare_exchange_impl<R, H, F>(
        &mut self,
        key: R,
        mut exchange: F,
    ) -> Result<Option<u64>, H::PopError>
    where
        R: key::Read,
        H: cursor::History<'g, R>,
        F: FnMut(ribbit::Packed<Edge>) -> ribbit::Packed<Edge>,
    {
        let mut cursor = Cursor::<R, H>::new(&mut self.smr, self.raw.root(), key);

        loop {
            let old = match cursor.traverse_exact() {
                None => return Ok(None),
                Some(Ok(old)) => old,
                Some(Err(())) => {
                    Self::freeze(&mut cursor, &key)?;
                    continue;
                }
            };

            if cursor
                .root()
                .compare_exchange_packed(old, exchange(old), Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return if old.meta().leaf() {
                    Ok(Some(old.data().into_leaf()))
                } else {
                    validate!(old.is_null());
                    Ok(None)
                };
            }
        }
    }

    #[inline]
    pub(crate) fn insert<R: key::Read>(&mut self, key: R, value: u64) -> Option<u64> {
        match self.insert_optimistic(key, value) {
            Ok(old) => old,
            Err(()) => self.insert_pessimistic(key, value),
        }
    }

    #[inline]
    fn insert_optimistic<R: key::Read>(&mut self, key: R, value: u64) -> Result<Option<u64>, ()> {
        self.insert_impl::<_, cursor::Optimistic<R>>(key, value)
    }

    #[cold]
    fn insert_pessimistic<R: key::Read>(&mut self, key: R, value: u64) -> Option<u64> {
        stat::increment(stat::Counter::InsertPessimistic);
        self.insert_impl::<_, cursor::Pessimistic<R>>(key, value)
            .unwrap()
    }

    #[inline]
    fn insert_impl<R: key::Read, H: cursor::History<'g, R>>(
        &mut self,
        key: R,
        value: u64,
    ) -> Result<Option<u64>, H::PopError> {
        let mut cursor = Cursor::<R, H>::new(&mut self.smr, self.raw.root(), key);

        loop {
            let (op, old, new) = match cursor.traverse_or_insert(value) {
                Ok(cas) => cas,
                Err(()) => {
                    Self::freeze(&mut cursor, &key)?;
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
                        return Ok(Some(old.data().into_leaf()));
                    } else {
                        validate!(old.is_null());
                        return Ok(None);
                    }
                }
                Ok(_) => {
                    stat::increment(op);
                    unsafe { Self::retire(&mut cursor, op, &key, old) };
                }
                Err(_) => {
                    // Does not go through SMR because `new` is still thread-local
                    unsafe { Self::deallocate(op, new) };
                }
            }
        }
    }

    #[cold]
    fn freeze<R: key::Read, H: cursor::History<'g, R>>(
        cursor: &mut Cursor<'g, '_, R, H>,
        key: &R,
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

            let (op, new) = node.replace(edge);

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
    unsafe fn deallocate(op: Op, edge: ribbit::Packed<Edge>) {
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
    unsafe fn retire<R: key::Read, H: cursor::History<'g, R>>(
        cursor: &mut Cursor<'g, '_, R, H>,
        op: Op,
        key: &R,
        edge: ribbit::Packed<Edge>,
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
    ) -> PrefixNonLinearizable<'g, 'l, W, S>
    where
        R: key::Read,
        W: key::Write + From<R>,
        S: crate::iter::Sort,
    {
        let mut cursor =
            Cursor::<R, cursor::Optimistic<_>>::new(&mut self.smr, self.raw.root(), prefix);

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
    ) -> RangeNonLinearizableIter<'g, 'l, R, W>
    where
        R: key::Read,
        W: key::Write<Len = usize> + PartialOrd<R> + From<R>,
    {
        let prefix = min.prefix(&max);

        let mut cursor =
            Cursor::<R, cursor::Optimistic<_>>::new(&mut self.smr, self.raw.root(), prefix);

        let iter = match cursor.traverse_prefix() {
            Some(_) => unsafe {
                iter::RangeIter::<R, W>::new(
                    cursor.root(),
                    W::from(prefix.slice(cursor.bit())),
                    min,
                    max,
                )
            },
            None => iter::RangeIter::<R, W>::empty(),
        };

        RangeNonLinearizableIter {
            iter,
            _guard: cursor.into_guard(),
        }
    }

    pub(crate) fn range<'l, K: Key, V: Value>(
        &'l mut self,
        min: K::Read<'l>,
        max: K::Read<'l>,
        output: &mut Vec<(K, V)>,
    ) {
        // FIXME: deduplicate prefix traversal?
        let prefix = min.prefix(&max);

        let mut cursor = Cursor::<K::Read<'l>, cursor::Optimistic<_>>::new(
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
        .for_each(|key, value| output.push((K::from(key.clone()), V::from_u64(value))));

        for retry in 0.. {
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
                        let new_key = K::from(new_writer.clone());
                        output.insert(index, (new_key, new_value));
                    }
                };
            });

            if len == output.len() && !dirty {
                stat::record(stat::Record::RangeConflict, retry);
                return;
            }

            crate::cold();
            validate!(output.len() <= len);
            output.truncate(len);
        }

        unsafe { core::hint::unreachable_unchecked() }
    }
}

pub(crate) struct RangeNonLinearizableIter<'g, 'l, R, W> {
    iter: iter::RangeIter<'g, R, W>,
    _guard: smr::Guard<'g, 'l>,
}

impl<'g, 'l, R, W> RangeNonLinearizableIter<'g, 'l, R, W>
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

pub(crate) struct PrefixNonLinearizable<'g, 'l, W: key::Write, S: crate::iter::Sort> {
    iter: iter::LeafIter<'g, W, S>,
    _guard: smr::Guard<'g, 'l>,
}

impl<'g, 'l, W, S> PrefixNonLinearizable<'g, 'l, W, S>
where
    W: key::Write,
    S: crate::iter::Sort,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(&W, u64)> {
        self.iter.lend()
    }
}
