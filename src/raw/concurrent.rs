use core::sync::atomic::Ordering;

use crate::byte;
use crate::edge;
use crate::key;
use crate::node;
use crate::raw::cursor;
use crate::raw::iter;
use crate::raw::sequential;
use crate::raw::Cursor;
use crate::raw::Op;
use crate::smr;
use crate::stat;
use crate::Edge;

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
    pub(crate) fn get<R: key::Read>(&self, key: R) -> Option<u64> {
        let _guard = self.smr.protect_read(key.peek_all());

        let mut root = self.raw.root();
        let mut key = key;
        loop {
            let edge = root.load_packed(Ordering::Relaxed);
            let meta = edge.meta();

            let _ = meta.key().match_exact(&mut key)?;
            let data = edge.data();

            if meta.leaf() {
                return Some(edge.data());
            } else if data == 0 {
                return None;
            } else {
                let byte = key.next()?;
                let data = edge.data();
                let node = unsafe { Edge::next_node_unchecked(data) };
                root = node.get(byte)?;
            }
        }
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
        let mut guard = self.smr.protect_write(key.peek_all());
        let mut cursor = Cursor::<R, H>::new(key, self.raw.root());

        loop {
            let old = match cursor.traverse_exact() {
                None => return Ok(None),
                Some(Ok(old)) => old,
                Some(Err(())) => {
                    Self::freeze(&mut guard, &mut cursor, &key)?;
                    continue;
                }
            };

            if cursor
                .root()
                .compare_exchange_packed(old, exchange(old), Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return if old.meta().leaf() {
                    Ok(Some(old.data()))
                } else {
                    validate_eq!(old.data(), 0);
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
        let mut guard = self.smr.protect_write(key.peek_all());
        let mut cursor = Cursor::<R, H>::new(key, self.raw.root());

        loop {
            let (op, old, new) = match cursor.traverse_or_insert(value) {
                Ok(cas) => cas,
                Err(()) => {
                    Self::freeze(&mut guard, &mut cursor, &key)?;
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
                        return Ok(Some(old.data()));
                    } else {
                        validate_eq!(old.data(), 0);
                        return Ok(None);
                    }
                }
                Ok(_) => {
                    stat::increment(op);
                    unsafe { Self::retire(&mut guard, &cursor, op, &key, old) };
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
        guard: &mut smr::WriteGuard,
        cursor: &mut Cursor<'g, R, H>,
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

            // Already helped by another thread
            if meta.leaf() || node.as_data() != data {
                return Ok(());
            }

            let (op, new) = node.replace(meta);

            match cursor.root().compare_exchange_packed(
                edge,
                new,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => unsafe {
                    stat::increment(op);
                    Self::retire(guard, cursor, Op::Node(op), key, edge);
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
                Edge::deallocate_unchecked(edge, stat::Counter::FreeConflict)
            },
        }
    }

    #[cold]
    unsafe fn retire<R: key::Read, H: cursor::History<'g, R>>(
        guard: &mut smr::WriteGuard,
        cursor: &Cursor<'g, R, H>,
        op: Op,
        key: &R,
        edge: ribbit::Packed<Edge>,
    ) {
        match op {
            Op::Edge(_) => return,
            Op::Node(_) => (),
        }

        let prefix = key.peek(byte::Len::MAX.min_bits(cursor.bit()));

        unsafe {
            guard.retire(edge.with_meta(edge.meta().with_key(prefix)));
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
        let guard = self.smr.protect_read(prefix.peek_all());
        let iter = match unsafe { self.traverse_prefix::<R, W>(prefix) } {
            Some((writer, root)) => unsafe { iter::LeafIter::new(root, writer) },
            None => iter::LeafIter::empty(),
        };

        PrefixNonLinearizable {
            iter,
            _guard: guard,
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
        let guard = self.smr.protect_read(prefix.peek_all());
        let iter = match unsafe { self.traverse_prefix::<R, W>(prefix) } {
            Some((writer, root)) => unsafe { iter::RangeIter::<R, W>::new(root, writer, min, max) },
            None => iter::RangeIter::<R, W>::empty(),
        };

        RangeNonLinearizableIter {
            iter,
            _guard: guard,
        }
    }

    pub(crate) fn range<R, W>(&mut self, min: R, max: R) -> std::vec::IntoIter<(W, u64)>
    where
        R: key::Read,
        W: key::Write<Len = usize> + PartialOrd<R> + From<R>,
    {
        // FIXME: deduplicate prefix traversal?
        let prefix = min.prefix(&max);
        let _guard = self.smr.protect_read(prefix.peek_all());
        let Some((writer, root)) = (unsafe { self.traverse_prefix::<R, W>(prefix) }) else {
            return Vec::new().into_iter();
        };

        let mut prev: Option<Vec<(W, u64)>> = None;
        let mut count = 0;
        loop {
            count += 1;

            let mut iter = unsafe { iter::RangeIter::<R, W>::new(root, writer.clone(), min, max) };

            let next = core::iter::from_fn(|| iter.lend().map(|(key, value)| (key.clone(), value)))
                .collect::<Vec<_>>();

            match prev {
                Some(prev) if prev == next => {
                    stat::record(stat::Record::RangeConflict, count);
                    return next.into_iter();
                }
                None | Some(_) => prev = Some(next),
            }
        }
    }

    #[inline]
    unsafe fn traverse_prefix<R, W>(&self, prefix: R) -> Option<(W, ribbit::Packed<Edge>)>
    where
        R: key::Read,
        W: key::Write + From<R>,
    {
        let mut reader = prefix;
        let mut bits = 0;
        let mut edge = self.raw.root().load_packed(Ordering::Acquire);

        loop {
            let meta = edge.meta();
            let data = edge.data();

            match meta.key().match_prefix(&mut reader)? {
                byte::MatchPrefix::Full(len) if !meta.leaf() && data != 0 => {
                    let node = unsafe { Edge::next_node_unchecked(data) };
                    let Some(byte) = reader.next() else { break };
                    let next = node.get(byte)?;

                    edge = next.load_packed(Ordering::Acquire);
                    bits += len.bits() + 8;
                }
                byte::MatchPrefix::Full(_) | byte::MatchPrefix::Partial => break,
            }
        }

        Some((W::from(prefix.slice(bits as usize)), edge))
    }
}

pub(crate) struct RangeNonLinearizableIter<'g, 'l, R, W> {
    iter: iter::RangeIter<'g, R, W>,
    _guard: smr::ReadGuard<'g, 'l>,
}

impl<'g, 'l, R, W> RangeNonLinearizableIter<'g, 'l, R, W>
where
    R: key::Read,
    W: key::Write<Len = usize> + PartialOrd<R>,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(&W, u64)> {
        self.iter.lend()
    }
}

pub(crate) struct PrefixNonLinearizable<'g, 'l, W: key::Write, S: crate::iter::Sort> {
    iter: iter::LeafIter<'g, W, S>,
    _guard: smr::ReadGuard<'g, 'l>,
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
