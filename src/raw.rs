use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;
use ribbit::Pack as _;
use ribbit::Unpack as _;

use crate::byte;
use crate::cursor;
use crate::cursor::Cursor;
use crate::cursor::Op;
use crate::edge;
use crate::node;
use crate::smr;
use crate::stat;
use crate::Edge;

#[derive(Default)]
pub(crate) struct Raw {
    smr: smr::Global,
    root: Atomic128<Edge>,
}

impl Raw {
    #[inline]
    pub(crate) fn pin(&self) -> Ref {
        Ref {
            smr: self.smr.pin(),
            root: &self.root,
        }
    }
}

pub(crate) struct Ref<'a> {
    smr: smr::Local<'a>,
    root: &'a Atomic128<Edge>,
}

impl Ref<'_> {
    #[inline]
    pub(crate) fn insert<K: byte::Iterator>(&mut self, key: K, value: u64) -> Option<u64> {
        match self.insert_optimistic(key.clone(), value) {
            Ok(old) => old,
            Err(()) => self.insert_pessimistic(key, value),
        }
    }

    #[inline]
    fn insert_optimistic<K: byte::Iterator>(
        &mut self,
        key: K,
        value: u64,
    ) -> Result<Option<u64>, ()> {
        self.insert_impl::<_, cursor::Optimistic<K>>(key, value)
    }

    #[cold]
    fn insert_pessimistic<K: byte::Iterator>(&mut self, key: K, value: u64) -> Option<u64> {
        stat::increment(stat::Counter::InsertPessimistic);
        self.insert_impl::<_, cursor::Pessimistic<K>>(key, value)
            .unwrap()
    }

    #[inline]
    fn insert_impl<'a, K: byte::Iterator, P: cursor::History<'a, K>>(
        &'a mut self,
        key: K,
        value: u64,
    ) -> Result<Option<u64>, P::PopError> {
        let mut guard = self.smr.protect_write(key.peek(byte::Array::MAX_LEN));

        let mut cursor = Cursor::<K, P>::new(key.clone(), self.root);

        loop {
            let (op, old, new) = cursor.traverse_or_insert(value);

            if old.meta().frozen() {
                cursor.pop()?;
                continue;
            }

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
                    unsafe { Self::retire(&mut guard, &key, &cursor, op, old) };
                }
                Err(edge) => {
                    // Does not go through EBR because `new` is still thread-local
                    unsafe { Self::deallocate(op, new) };

                    if edge.meta().frozen() {
                        cursor.pop()?;
                    }
                }
            }
        }
    }

    #[cold]
    unsafe fn retire<'a, K: byte::Iterator, P: cursor::History<'a, K>>(
        guard: &mut smr::WriteGuard,
        key: &K,
        cursor: &Cursor<'a, K, P>,
        op: cursor::Op,
        edge: ribbit::Packed<Edge>,
    ) {
        match op {
            cursor::Op::Edge(_) => return,
            cursor::Op::Node(_) => (),
        }

        let index = cursor.index();
        let prefix = key.peek(byte::Array::min_len(index, byte::Array::MAX_LEN));

        unsafe {
            guard.retire(edge.with_meta(edge.meta().with_key(prefix)));
        }
    }

    #[cold]
    unsafe fn deallocate(op: cursor::Op, edge: ribbit::Packed<Edge>) {
        match op {
            cursor::Op::Node(node::Op::Destroy | node::Op::Compress)
            | cursor::Op::Edge(edge::Op::Insert | edge::Op::Remove) => return,

            cursor::Op::Node(node::Op::Grow | node::Op::Replace | node::Op::Shrink)
            | cursor::Op::Edge(edge::Op::Create | edge::Op::Expand) => (),
        }

        unsafe { Edge::deallocate(edge, stat::Counter::FreeConflict) }
    }

    #[inline]
    pub(crate) fn get<K: byte::Iterator>(&self, key: K) -> Option<u64> {
        let _guard = self.smr.protect_read(key.peek(byte::Array::MAX_LEN));

        let mut root = self.root;
        let mut key = key;
        loop {
            let edge = root.load_packed(Ordering::Relaxed);
            let meta = edge.meta();
            let _ = byte::Array::match_prefix(&mut key, meta.key())?;

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
    pub(crate) fn remove<K: byte::Iterator>(&mut self, key: K) -> Option<u64> {
        let _guard = self.smr.protect_write(key.peek(byte::Array::MAX_LEN));

        let mut cursor = Cursor::<K, cursor::Optimistic<K>>::new(key, self.root);
        let mut old = cursor.traverse_exact()?;

        loop {
            if old.meta().frozen() {
                todo!()
            }

            match cursor.root().compare_exchange_packed(
                old,
                old.with_meta(old.meta().with_kind(node::Kind::None.pack())),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(edge) if matches!(edge.meta().kind().unpack(), node::Kind::None) => {
                    return None
                }
                Err(edge) if edge.meta() != old.meta() => todo!(
                    "Handle metadata conflict in remove: expected {:?} but found {:?}",
                    old.meta(),
                    edge.meta(),
                ),
                Err(edge) => {
                    old = edge;
                }
            }
        }

        Some(old.data())
    }

    #[inline]
    pub(crate) fn update<K: byte::Iterator>(&mut self, key: K, value: u64) -> Option<u64> {
        let _guard = self.smr.protect_write(key.peek(byte::Array::MAX_LEN));

        let mut cursor = Cursor::<K, cursor::Optimistic<K>>::new(key, self.root);
        let mut old = cursor.traverse_exact()?;

        loop {
            if old.meta().frozen() {
                todo!()
            }

            match cursor.root().compare_exchange_packed(
                old,
                Edge::new_leaf(old.meta().key(), value),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,

                Err(edge)
                    if edge.meta().frozen()
                        || edge.meta().key() != old.meta().key()
                        || !matches!(
                            edge.meta().kind().unpack(),
                            node::Kind::None | node::Kind::Leaf
                        ) =>
                {
                    todo!(
                        "Handle metadata conflict in update: expected {:?} but found {:?}",
                        old.meta(),
                        edge.meta(),
                    )
                }
                Err(edge) => {
                    old = edge;
                }
            }
        }

        Some(old.data())
    }
    //
    // pub fn iter(&mut self) -> impl Iterator<Item = (Rc<Vec<u8>>, u64)> + '_ {
    //     self.preorder()
    //         .filter_map(|(_, key, edge)| match edge.meta().kind().unpack() {
    //             node::Kind::None | node::Kind::Node3 | node::Kind::Node15 | node::Kind::Node256 => {
    //                 None
    //             }
    //             node::Kind::Leaf => Some((key, edge.data())),
    //         })
    // }
    //
    // pub fn keys(&mut self) -> impl Iterator<Item = Rc<Vec<u8>>> + '_ {
    //     self.iter().map(|(key, _)| key)
    // }
    //
    // pub fn values(&mut self) -> impl Iterator<Item = u64> + '_ {
    //     self.iter().map(|(_, value)| value)
    // }

    // pub(crate) fn preorder(
    //     &mut self,
    // ) -> impl Iterator<Item = (usize, Rc<Vec<u8>>, ribbit::Packed<Edge>)> + '_ {
    //     EntryIter::new(&mut self.root)
    // }

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
    // pub fn range<'r, R: RangeBounds<&'r K> + 'r>(&self, range: R) -> impl Iterator<Item = u64> + 'r
    // where
    //     K: 'r,
    // {
    //     let low = range.start_bound().map(|low| low);
    //     let high = range.end_bound().map(|high| high);
    //
    //     let prefix = match (low, high) {
    //         (Bound::Unbounded, _) | (_, Bound::Unbounded) => &[],
    //         (
    //             Bound::Included(low) | Bound::Excluded(low),
    //             Bound::Included(high) | Bound::Excluded(high),
    //         ) => {
    //             let prefix = low
    //                 .iter()
    //                 .zip(high)
    //                 .position(|(left, right)| left != right)
    //                 .unwrap_or_else(|| low.len().min(high.len()));
    //             &low[..prefix]
    //         }
    //     };
    //
    //     core::iter::empty()
    //     // let mut cursor = Cursor::<K, cursor::Optimistic>::new(&self.root, prefix);
    //     // let Some((len, _)) = cursor.traverse_prefix() else {
    //     //     return Or::L(None.into_iter());
    //     // };
    //     //
    //     // let iter = ScanIter::new(
    //     //     low.map(|low| &low[len..]),
    //     //     high.map(|high| &high[len..]),
    //     //     cursor.here(),
    //     // );
    //     //
    //     // match iter {
    //     //     Or::L(leaf) => Or::L(leaf.into_iter()),
    //     //     Or::R(iter) => Or::R(
    //     //         // FIXME: root node can contain leaves outside of bounds
    //     //         iter.flat_map(|node| {
    //     //             unsafe { node.iter() }.filter_map(|(_, edge)| {
    //     //                 let edge = edge.load(Ordering::Relaxed);
    //     //                 if matches!(edge.meta.kind, node::Kind::Leaf) {
    //     //                     Some(edge.data)
    //     //                 } else {
    //     //                     None
    //     //                 }
    //     //             })
    //     //         })
    //     //         .collect::<Vec<_>>()
    //     //         .into_iter(),
    //     //     ),
    //     // }
    // }
}

// struct EntryIter<'a> {
//     // Workaround for lending iterator
//     // https://users.rust-lang.org/t/how-to-write-an-iterator-that-returns-references-to-itself/72386/5
//     key: Rc<Vec<u8>>,
//
//     // TODO: allow starting traversal at a given prefix?
//     frontier: Vec<(usize, Or<EdgeIter<'a>, NodeIter<'a>>)>,
// }
//
// type EdgeIter<'a> = iter::Peekable<iter::Zip<iter::Once<bool>, node::EdgeIter<'a>>>;
// type NodeIter<'a> = iter::Peekable<iter::Zip<iter::Repeat<bool>, node::Iter<'a>>>;
//
// impl<'a> EntryIter<'a> {
//     fn new(root: &'a mut Atomic128<Edge>) -> Self {
//         Self {
//             key: Rc::new(Vec::new()),
//             frontier: vec![(
//                 0,
//                 Or::L(iter::zip(iter::once(false), core::slice::from_ref(root).iter()).peekable()),
//             )],
//         }
//     }
// }
//
// impl<'a> Iterator for EntryIter<'a> {
//     type Item = (usize, Rc<Vec<u8>>, ribbit::Packed<Edge>);
//
//     fn next(&mut self) -> Option<Self::Item> {
//         'vertical: loop {
//             // NOTE: we use `saturating_sub` to avoid underflow.
//             //
//             // If `self.frontier.len()` == 0, we will immediately return at `last_mut()`.
//             // We can't move the len call after because `self.frontier` is mutably borrowed.
//             let depth = self.frontier.len().saturating_sub(1);
//             let (len, iter) = self.frontier.last_mut()?;
//
//             'horizontal: loop {
//                 let Some((descend, byte, edge)) = (match iter {
//                     Or::L(iter_root) => iter_root
//                         .peek_mut()
//                         .map(|(descend, edge)| (descend, None, edge)),
//                     Or::R(iter_node) => iter_node
//                         .peek_mut()
//                         .map(|(descend, (key, edge))| (descend, Some(*key), edge)),
//                 }) else {
//                     Rc::make_mut(&mut self.key).truncate(*len);
//                     self.frontier.pop();
//                     continue 'vertical;
//                 };
//
//                 let edge = edge.load_packed(Ordering::Relaxed);
//                 let meta = edge.meta();
//                 let kind = meta.kind();
//
//                 // Skip empty edges
//                 if kind == node::Kind::NONE {
//                     iter.skip();
//                     continue 'horizontal;
//                 }
//
//                 // Update key for current edge
//                 let key = Rc::make_mut(&mut self.key);
//
//                 let edge_key = meta.key().unpack();
//
//                 // Produce edge before traversing for preorder traversal
//                 if !mem::replace(descend, true) {
//                     key.extend(byte.into_iter().chain(edge_key.bytes()));
//                     return Some((depth, Rc::clone(&self.key), edge));
//                 }
//
//                 iter.skip();
//                 let len = key.len() - edge_key.len.value() as usize - byte.is_some() as usize;
//
//                 if kind == node::Kind::LEAF {
//                     key.truncate(len);
//                     continue 'horizontal;
//                 } else {
//                     let node = unsafe { Edge::next_node_unchecked(edge.data(), kind) };
//                     self.frontier.push((
//                         len,
//                         Or::R(iter::repeat(false).zip(unsafe { node.iter() }).peekable()),
//                     ));
//                     continue 'vertical;
//                 }
//             }
//         }
//     }
// }
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
