mod cursor;

use core::ops::Bound;
use core::ops::RangeBounds;
use core::sync::atomic::Ordering;

use crate::edge;
use crate::key;
use crate::node;
use crate::raw::iter;
use crate::raw::sequential;
use crate::raw::Op;
use crate::smr;
use crate::stat;
use crate::Edge;
use cursor::Cursor;

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
            let _ = meta.key().match_prefix(&mut key)?;

            let kind = meta.kind();
            if kind >= node::Kind::NODE_3 {
                let byte = key.next()?;
                let data = edge.data();
                let node = unsafe { Edge::next_node_unchecked(data, kind) };
                root = node.get(byte)?;
            } else if kind == node::Kind::LEAF {
                return Some(edge.data());
            } else {
                validate_eq!(kind, node::Kind::NONE);
                return None;
            }
        }
    }

    #[inline]
    pub(crate) fn update<R: key::Read>(&mut self, key: R, value: u64) -> Option<u64> {
        match self.update_optimistic(key.clone(), value) {
            Ok(old) => old,
            Err(()) => self.update_pessimistic(key, value),
        }
    }

    #[inline]
    fn update_optimistic<R: key::Read>(&mut self, key: R, value: u64) -> Result<Option<u64>, ()> {
        self.update_impl::<_, cursor::Optimistic<R>>(key, value)
    }

    #[cold]
    fn update_pessimistic<R: key::Read>(&mut self, key: R, value: u64) -> Option<u64> {
        self.update_impl::<_, cursor::Pessimistic<R>>(key, value)
            .unwrap()
    }

    #[inline]
    fn update_impl<R: key::Read, H: cursor::History<'g, R>>(
        &mut self,
        key: R,
        value: u64,
    ) -> Result<Option<u64>, H::PopError> {
        let mut guard = self.smr.protect_write(key.peek_all());
        let mut cursor = Cursor::<R, H>::new(key, self.raw.root());

        loop {
            let old = match cursor.traverse_exact() {
                Ok(None) => return Ok(None),
                Ok(Some(old)) => old,
                Err(()) => {
                    Self::freeze(&mut guard, &mut cursor)?;
                    continue;
                }
            };

            validate!(!old.meta().frozen());

            if cursor
                .root()
                .compare_exchange_packed(
                    old,
                    Edge::new_leaf(old.meta().key(), value),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                return if old.meta().kind() == node::Kind::LEAF {
                    Ok(Some(old.data()))
                } else {
                    validate_eq!(old.meta().kind(), node::Kind::NONE);
                    Ok(None)
                };
            }
        }
    }

    #[inline]
    pub(crate) fn remove<R: key::Read>(&mut self, key: R) -> Option<u64> {
        match self.remove_optimistic(key.clone()) {
            Ok(old) => old,
            Err(()) => self.remove_pessimistic(key),
        }
    }

    #[inline]
    fn remove_optimistic<R: key::Read>(&mut self, key: R) -> Result<Option<u64>, ()> {
        self.remove_impl::<_, cursor::Optimistic<R>>(key)
    }

    #[cold]
    fn remove_pessimistic<R: key::Read>(&mut self, key: R) -> Option<u64> {
        self.remove_impl::<_, cursor::Pessimistic<R>>(key).unwrap()
    }

    #[inline]
    fn remove_impl<R: key::Read, H: cursor::History<'g, R>>(
        &mut self,
        key: R,
    ) -> Result<Option<u64>, H::PopError> {
        let mut guard = self.smr.protect_write(key.peek_all());
        let mut cursor = Cursor::<R, H>::new(key, self.raw.root());

        loop {
            let old = match cursor.traverse_exact() {
                Ok(None) => return Ok(None),
                Ok(Some(old)) => old,
                Err(()) => {
                    Self::freeze(&mut guard, &mut cursor)?;
                    continue;
                }
            };

            validate!(!old.meta().frozen());

            if cursor
                .root()
                .compare_exchange_packed(old, Edge::DEFAULT, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return if old.meta().kind() == node::Kind::LEAF {
                    Ok(Some(old.data()))
                } else {
                    validate_eq!(old.meta().kind(), node::Kind::NONE);
                    Ok(None)
                };
            }
        }
    }

    #[inline]
    pub(crate) fn insert<R: key::Read>(&mut self, key: R, value: u64) -> Option<u64> {
        match self.insert_optimistic(key.clone(), value) {
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
        let mut cursor = Cursor::<R, H>::new(key.clone(), self.raw.root());

        loop {
            let (op, old, new) = match cursor.traverse_or_insert(value) {
                Ok(cas) => cas,
                Err(()) => {
                    Self::freeze(&mut guard, &mut cursor)?;
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
                    if old.meta().kind() == node::Kind::NONE {
                        return Ok(None);
                    } else {
                        validate_eq!(old.meta().kind(), node::Kind::LEAF);
                        return Ok(Some(old.data()));
                    }
                }
                Ok(_) => {
                    stat::increment(op);
                    unsafe { Self::retire(&mut guard, &cursor, op, old) };
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
    ) -> Result<(), H::PopError> {
        let mut node = cursor.pop()?;
        let mut edge = cursor.root().load_packed(Ordering::Acquire);

        loop {
            while edge.meta().frozen() {
                node = cursor.pop()?;
                edge = cursor.root().load_packed(Ordering::Acquire);
            }
            let meta = edge.meta();
            let kind = meta.kind();

            // Already helped by another thread
            if kind < node::Kind::NODE_3 || node.as_u64() != edge.data() {
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
                    Self::retire(guard, cursor, Op::Node(op), edge);
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
                Edge::deallocate(edge, stat::Counter::FreeConflict)
            },
        }
    }

    #[cold]
    unsafe fn retire<R: key::Read, H: cursor::History<'g, R>>(
        guard: &mut smr::WriteGuard,
        cursor: &Cursor<'g, R, H>,
        op: Op,
        edge: ribbit::Packed<Edge>,
    ) {
        match op {
            Op::Edge(_) => return,
            Op::Node(_) => (),
        }

        unsafe {
            guard.retire(edge.with_meta(edge.meta().with_key(cursor.prefix())));
        }
    }

    pub(crate) fn range_non_linearizable<'l, B, K, R, W>(
        &'l mut self,
        range: B,
    ) -> RangeNonLinearizableIter<'g, 'l, B, K, W>
    where
        B: RangeBounds<K> + Clone,
        K: Copy,
        R: key::Read + From<K>,
        W: key::Write + PartialOrd<K> + From<R>,
    {
        let prefix = Self::prefix::<B, K, R>(range.clone());
        let guard = self.smr.protect_read(prefix.peek_all());
        let mut cursor = Cursor::<R, cursor::Optimistic<R>>::new(prefix.clone(), self.raw.root());
        let index = cursor.traverse_prefix();
        let mut stack = W::from(prefix);
        stack.truncate(index);

        let iter = unsafe {
            iter::LeafIter::<B, K, W, node::SortedIter>::new(cursor.root(), stack, range)
        };

        RangeNonLinearizableIter {
            iter,
            _guard: guard,
        }
    }

    pub(crate) fn range<B, K, R, W>(&mut self, range: B) -> std::vec::IntoIter<(W, u64)>
    where
        B: RangeBounds<K> + Clone,
        K: Copy,
        R: key::Read + From<K>,
        W: key::Write + PartialOrd<K> + From<R>,
    {
        // FIXME: deduplicate prefix traversal?
        let prefix = Self::prefix::<B, K, R>(range.clone());
        let _guard = self.smr.protect_read(prefix.peek_all());
        let mut cursor = Cursor::<R, cursor::Optimistic<R>>::new(prefix.clone(), self.raw.root());
        let index = cursor.traverse_prefix();
        let mut stack = W::from(prefix);
        stack.truncate(index);

        let mut prev: Option<Vec<(W, u64)>> = None;
        let mut count = 0;
        loop {
            count += 1;

            let mut iter = unsafe {
                iter::LeafIter::<B, K, W, node::SortedIter>::new(
                    cursor.root(),
                    stack.clone(),
                    range.clone(),
                )
            };

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

    fn prefix<B: RangeBounds<K>, K, R>(range: B) -> R
    where
        K: Copy,
        R: key::Read + From<K>,
    {
        match (range.start_bound(), range.end_bound()) {
            (Bound::Unbounded, _) | (_, Bound::Unbounded) => R::default(),
            (
                Bound::Included(start) | Bound::Excluded(start),
                Bound::Included(end) | Bound::Excluded(end),
            ) => {
                let start = R::from(*start);
                let end = R::from(*end);
                start.prefix(&end)
            }
        }
    }
}

pub(crate) struct RangeNonLinearizableIter<'g, 'l, B, K, W> {
    iter: iter::LeafIter<'g, B, K, W, node::SortedIter<'g>>,
    _guard: smr::ReadGuard<'g, 'l>,
}

impl<'g, 'l, B, K, W> RangeNonLinearizableIter<'g, 'l, B, K, W>
where
    B: RangeBounds<K>,
    W: key::Write + PartialOrd<K>,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(&W, u64)> {
        self.iter.lend()
    }
}
