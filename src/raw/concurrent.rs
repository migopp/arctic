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

pub(crate) struct MapRef<'a> {
    smr: smr::Local<'a>,
    raw: &'a sequential::Map,
}

impl<'a> MapRef<'a> {
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
    fn update_impl<R: key::Read, H: cursor::History<'a, R>>(
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
    fn remove_impl<R: key::Read, H: cursor::History<'a, R>>(
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
    fn insert_impl<R: key::Read, H: cursor::History<'a, R>>(
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
    fn freeze<R: key::Read, H: cursor::History<'a, R>>(
        guard: &mut smr::WriteGuard,
        cursor: &mut Cursor<'a, R, H>,
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
    unsafe fn retire<R: key::Read, H: cursor::History<'a, R>>(
        guard: &mut smr::WriteGuard,
        cursor: &Cursor<'a, R, H>,
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

    pub(crate) fn range_non_linearizable<B, R, W>(
        &mut self,
        range: B,
    ) -> iter::Iter<'a, W, iter::SelectRange<B, R, W>, iter::Preorder, node::SortedIter<'a>>
    where
        B: RangeBounds<R>,
        R: key::Read + PartialOrd<W>,
        W: key::Write + PartialOrd<R> + From<R>,
    {
        let prefix = match (range.start_bound(), range.end_bound()) {
            (Bound::Unbounded, _) | (_, Bound::Unbounded) => R::default(),
            (
                Bound::Included(low) | Bound::Excluded(low),
                Bound::Included(high) | Bound::Excluded(high),
            ) => low.prefix(high),
        };

        let _guard = self.smr.protect_read(prefix.peek_all());
        let mut cursor = Cursor::<R, cursor::Optimistic<R>>::new(prefix.clone(), self.raw.root());
        let index = cursor.traverse_prefix();
        let mut stack = W::from(prefix);
        stack.truncate(index);

        unsafe {
            iter::Iter::<W, iter::SelectRange<B, R, W>, iter::Preorder, node::SortedIter>::new(
                cursor.root(),
                stack,
                iter::SelectRange::new(range),
            )
        }
    }

    // pub fn scan(&self, low: &K, count: usize) -> impl Iterator<Item = u64> {
    //     let iter = ScanIter::new(Bound::Included(low), Bound::Unbounded, &self.root);
    //
    //     match iter {
    //         Or::L(leaf) => Or::L(leaf.into_iter()),
    //         Or::R(iter) => Or::R(
    //             iter.flat_map(|node| {
    //                 unsafe { node.iter() }.filter_map(|(_, edge)| {
    //                     let edge = edge.load(Ordering::Relaxed);
    //                     if matches!(edge.meta.kind, node::Kind::Leaf) {
    //                         Some(edge.data)
    //                     } else {
    //                         None
    //                     }
    //                 })
    //             })
    //             .take(count)
    //             .collect::<Vec<_>>()
    //             .into_iter(),
    //         ),
    //     }
    // }
    //
}

pub(crate) type RangeIter<'a, B, R, W> =
    iter::Iter<'a, W, iter::SelectRange<B, R, W>, iter::Preorder, node::SortedIter<'a>>;

//
// struct ScanIter<'a, K> {
//     window: Window<'a, K>,
//
//     // root: node::EdgeIter<'a>,
//     frontier: Vec<(usize, NodeIter<'a>)>,
//
//     _key: PhantomData<K>,
// }
//
// impl<'a, K: Key> ScanIter<'a, K> {
//     fn new(
//         low: Bound<&'a K>,
//         high: Bound<&'a K>,
//         root: &'a Atomic128<Edge>,
//     ) -> Or<Option<u64>, iter::Chain<iter::Once<node::Ref<'a>>, Self>> {
//         let edge = root.load_packed(Ordering::Acquire);
//         let meta = edge.meta();
//         let kind = meta.kind();
//
//         let node = if kind == node::Kind::NONE {
//             return Or::L(None);
//         } else if kind == node::Kind::LEAF {
//             return Or::L(Some(edge.data()));
//         } else {
//             unsafe { Edge::next_node_unchecked(edge.data(), kind) }
//         };
//
//         Or::R(iter::once(node).chain(Self {
//             window: Window {
//                 index: 0,
//                 low,
//                 high,
//                 within_low: match low {
//                     Bound::Unbounded => Within::Yes(0),
//                     _ => Within::Maybe,
//                 },
//                 within_high: match high {
//                     Bound::Unbounded => Within::Yes(0),
//                     _ => Within::Maybe,
//                 },
//             },
//             // root: node::EdgeIter::new(core::slice::from_ref(root)),
//             frontier: vec![(
//                 0,
//                 iter::repeat(false).zip(unsafe { node.iter() }).peekable(),
//             )],
//             _key: PhantomData,
//         }))
//     }
// }
//
// impl<'a> Iterator for ScanIter<'a> {
//     type Item = node::Ref<'a>;
//     fn next(&mut self) -> Option<Self::Item> {
//         'vertical: loop {
//             let (delta, iter) = self.frontier.last_mut()?;
//
//             'horizontal: loop {
//                 let Some((descend, (key, edge))) = iter.peek_mut() else {
//                     self.window.pop(*delta);
//                     self.frontier.pop();
//                     continue 'vertical;
//                 };
//
//                 let edge = edge.load_packed(Ordering::Relaxed);
//                 let meta = edge.meta();
//                 let kind = meta.kind();
//
//                 let node = if kind < node::Kind::NODE_3 {
//                     iter.next();
//                     continue 'horizontal;
//                 } else {
//                     unsafe { Edge::next_node_unchecked(edge.data(), kind) }
//                 };
//
//                 if !meta
//                     .key()
//                     .unpack()
//                     .with_bytes(Some(*key), |key| self.window.push(key))
//                 {
//                     iter.next();
//                     continue 'horizontal;
//                 }
//
//                 if !mem::replace(descend, true) {
//                     self.frontier.push((
//                         1 + edge.meta().key().len().value() as usize,
//                         iter::repeat(false).zip(unsafe { node.iter() }).peekable(),
//                     ));
//                     return Some(node);
//                 } else {
//                     iter.next();
//                     continue 'vertical;
//                 }
//             }
//         }
//     }
// }
//
// #[derive(Debug)]
// struct Window<'a, K> {
//     index: usize,
//     low: Bound<&'a K>,
//     high: Bound<&'a K>,
//     within_low: Within,
//     within_high: Within,
// }
//
// #[derive(Copy, Clone, Debug)]
// enum Within {
//     Yes(usize),
//     Maybe,
// }
//
// impl<'a, K> Window<'a, K> {
//     fn push(&mut self, key: &[u8]) -> bool {
//         if let (Within::Yes(_), Within::Yes(_)) = (self.within_low, self.within_high) {
//             self.index += key.len();
//             return true;
//         }
//
//         // Check against low
//         if matches!(self.within_low, Within::Maybe) {
//             match self.low.map(|low| &low[self.index..]) {
//                 Bound::Unbounded => {
//                     assert_eq!(self.index, 0);
//                     self.within_low = Within::Yes(self.index);
//                 }
//                 Bound::Included(low) if key.len() == low.len() => {
//                     if key < low {
//                         return false;
//                     }
//                 }
//                 Bound::Excluded(low) if key.len() == low.len() => {
//                     if key <= low {
//                         return false;
//                     }
//                 }
//
//                 Bound::Included(low) | Bound::Excluded(low) => {
//                     if key.len() < low.len() {
//                         match low[..key.len()].cmp(key) {
//                             cmp::Ordering::Less => self.within_low = Within::Yes(self.index),
//                             cmp::Ordering::Equal => (),
//                             cmp::Ordering::Greater => {
//                                 return false;
//                             }
//                         }
//                     } else {
//                         assert!(key.len() > low.len());
//                         self.within_low = Within::Yes(self.index);
//                     }
//                 }
//             }
//         }
//
//         // Check against high
//         if matches!(self.within_high, Within::Maybe) {
//             match self.high.map(|high| &high[self.index..]) {
//                 Bound::Unbounded => {
//                     assert_eq!(self.index, 0);
//                     self.within_high = Within::Yes(self.index);
//                 }
//                 Bound::Included(high) if key.len() == high.len() => {
//                     if key > high {
//                         return false;
//                     }
//                 }
//                 Bound::Excluded(high) if key.len() == high.len() => {
//                     if key >= high {
//                         return false;
//                     }
//                 }
//                 Bound::Included(high) | Bound::Excluded(high) => {
//                     if key.len() < high.len() {
//                         match high[..key.len()].cmp(key) {
//                             cmp::Ordering::Less => {
//                                 return false;
//                             }
//                             cmp::Ordering::Equal => (),
//                             cmp::Ordering::Greater => {
//                                 self.within_high = Within::Yes(self.index);
//                             }
//                         }
//                     } else {
//                         assert!(key.len() > high.len());
//                         return false;
//                     }
//                 }
//             }
//         }
//
//         self.index += key.len();
//         true
//     }
//
//     fn pop(&mut self, delta: usize) {
//         self.index -= delta;
//
//         match self.within_low {
//             Within::Yes(reset) if self.index == reset => self.within_low = Within::Maybe,
//             _ => (),
//         }
//
//         match self.within_high {
//             Within::Yes(reset) if self.index == reset => self.within_high = Within::Maybe,
//             _ => (),
//         }
//     }
// }
