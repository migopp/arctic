use core::cmp;
use core::iter;
use core::mem;
use core::ops::Bound;
use core::ops::RangeBounds;
use core::sync::atomic::Ordering;
use std::rc::Rc;

use ribbit::atomic::Atomic128;
use ribbit::Pack as _;
use ribbit::Unpack as _;

use crate::cursor;
use crate::cursor::Cursor;
use crate::cursor::Op;
use crate::edge;
use crate::node;
use crate::stat;
use crate::Edge;
use crate::Or;

#[derive(Default)]
pub struct Raw {
    root: Atomic128<Edge>,
}

impl Raw {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, key: &[u8], value: u64) -> Option<u64> {
        match self.insert_optimistic(key, value) {
            Ok(old) => old,
            Err(()) => self.insert_pessimistic(key, value),
        }
    }

    #[inline]
    fn insert_optimistic(&self, key: &[u8], value: u64) -> Result<Option<u64>, ()> {
        self.insert_impl::<cursor::Optimistic>(key, value)
    }

    #[cold]
    fn insert_pessimistic(&self, key: &[u8], value: u64) -> Option<u64> {
        stat::increment(stat::Counter::InsertPessimistic);
        self.insert_impl::<cursor::Pessimistic>(key, value).unwrap()
    }

    fn insert_impl<'a, P: cursor::History<'a>>(
        &'a self,
        key: &[u8],
        value: u64,
    ) -> Result<Option<u64>, P::PopError> {
        let mut cursor = Cursor::<P>::new(&self.root, key);

        loop {
            let (op, old, new) = cursor.traverse_or_insert(value);

            if old.meta().frozen() {
                cursor.pop()?;
                continue;
            }

            let edge = match cursor.here().compare_exchange_packed(
                old,
                new,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(edge) => {
                    stat::increment(op);
                    match (op, edge.meta().kind().unpack()) {
                        (Op::Edge(edge::Op::Insert), node::Kind::None) => return Ok(None),
                        (Op::Edge(edge::Op::Insert), node::Kind::Leaf) => {
                            return Ok(Some(edge.data()))
                        }
                        // FIXME: retire old allocation with SMR
                        _ => continue,
                    }
                }
                Err(edge) => edge,
            };

            match op {
                Op::Node(node::Op::Destroy | node::Op::Compress)
                | Op::Edge(edge::Op::Insert | edge::Op::Remove) => (),

                Op::Node(node::Op::Grow | node::Op::Replace | node::Op::Shrink)
                | Op::Edge(edge::Op::Create | edge::Op::Expand) => unsafe {
                    Edge::deallocate(new);
                },
            }

            if edge.meta().frozen() {
                cursor.pop()?;
            }
        }
    }

    pub fn get(&self, key: &[u8]) -> Option<u64> {
        Cursor::<cursor::Optimistic>::new(&self.root, key)
            .traverse_exact()
            .map(|edge| edge.data())
    }

    pub fn remove(&self, key: &[u8]) -> Option<u64> {
        let mut cursor = Cursor::<cursor::Optimistic>::new(&self.root, key);
        let mut old = cursor.traverse_exact()?;
        let edge = cursor.here();

        loop {
            if old.meta().frozen() {
                todo!()
            }

            match edge.compare_exchange_packed(
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

    pub fn update(&self, key: &[u8], value: u64) -> Option<u64> {
        let mut cursor = Cursor::<cursor::Optimistic>::new(&self.root, key);
        let mut old = cursor.traverse_exact()?;
        let edge = cursor.here();

        loop {
            if old.meta().frozen() {
                todo!()
            }

            match edge.compare_exchange_packed(
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

    pub fn iter(&mut self) -> impl Iterator<Item = (Rc<Vec<u8>>, u64)> + '_ {
        self.preorder()
            .filter_map(|(_, key, edge)| match edge.meta().kind().unpack() {
                node::Kind::None | node::Kind::Node3 | node::Kind::Node15 | node::Kind::Node256 => {
                    None
                }
                node::Kind::Leaf => Some((key, edge.data())),
            })
    }

    pub fn keys(&mut self) -> impl Iterator<Item = Rc<Vec<u8>>> + '_ {
        self.iter().map(|(key, _)| key)
    }

    pub fn values(&mut self) -> impl Iterator<Item = u64> + '_ {
        self.iter().map(|(_, value)| value)
    }

    pub(crate) fn preorder(
        &mut self,
    ) -> impl Iterator<Item = (usize, Rc<Vec<u8>>, ribbit::Packed<Edge>)> + '_ {
        EntryIter::new(&mut self.root)
    }

    pub fn scan(&self, low: &[u8], count: usize) -> impl Iterator<Item = u64> {
        let iter = ScanIter::new(Bound::Included(low), Bound::Unbounded, &self.root);

        match iter {
            Or::L(leaf) => Or::L(leaf.into_iter()),
            Or::R(iter) => Or::R(
                iter.flat_map(|node| {
                    unsafe { node.iter() }.filter_map(|(_, edge)| {
                        let edge = edge.load(Ordering::Relaxed);
                        if matches!(edge.meta.kind, node::Kind::Leaf) {
                            Some(edge.data)
                        } else {
                            None
                        }
                    })
                })
                .take(count)
                .collect::<Vec<_>>()
                .into_iter(),
            ),
        }
    }

    pub fn range<'r, R: RangeBounds<B> + 'r, B: AsRef<[u8]> + 'r>(
        &self,
        range: R,
    ) -> impl Iterator<Item = u64> + 'r {
        let low = range.start_bound().map(|low| low.as_ref());
        let high = range.end_bound().map(|high| high.as_ref());

        let prefix = match (low, high) {
            (Bound::Unbounded, _) | (_, Bound::Unbounded) => &[],
            (
                Bound::Included(low) | Bound::Excluded(low),
                Bound::Included(high) | Bound::Excluded(high),
            ) => {
                let prefix = low
                    .iter()
                    .zip(high)
                    .position(|(left, right)| left != right)
                    .unwrap_or_else(|| low.len().min(high.len()));
                &low[..prefix]
            }
        };

        let mut cursor = Cursor::<cursor::Optimistic>::new(&self.root, prefix);
        let Some((len, _)) = cursor.traverse_prefix() else {
            return Or::L(None.into_iter());
        };

        let iter = ScanIter::new(
            low.map(|low| &low[len..]),
            high.map(|high| &high[len..]),
            cursor.here(),
        );

        match iter {
            Or::L(leaf) => Or::L(leaf.into_iter()),
            Or::R(iter) => Or::R(
                // FIXME: root node can contain leaves outside of bounds
                iter.flat_map(|node| {
                    unsafe { node.iter() }.filter_map(|(_, edge)| {
                        let edge = edge.load(Ordering::Relaxed);
                        if matches!(edge.meta.kind, node::Kind::Leaf) {
                            Some(edge.data)
                        } else {
                            None
                        }
                    })
                })
                .collect::<Vec<_>>()
                .into_iter(),
            ),
        }
    }
}

struct EntryIter<'a> {
    // Workaround for lending iterator
    // https://users.rust-lang.org/t/how-to-write-an-iterator-that-returns-references-to-itself/72386/5
    key: Rc<Vec<u8>>,

    // TODO: allow starting traversal at a given prefix?
    frontier: Vec<(usize, Or<EdgeIter<'a>, NodeIter<'a>>)>,
}

type EdgeIter<'a> = iter::Peekable<iter::Zip<iter::Once<bool>, node::EdgeIter<'a>>>;
type NodeIter<'a> = iter::Peekable<iter::Zip<iter::Repeat<bool>, node::Iter<'a>>>;

impl<'a> EntryIter<'a> {
    fn new(root: &'a mut Atomic128<Edge>) -> Self {
        Self {
            key: Rc::new(Vec::new()),
            frontier: vec![(
                0,
                Or::L(iter::zip(iter::once(false), core::slice::from_ref(root).iter()).peekable()),
            )],
        }
    }
}

impl<'a> Iterator for EntryIter<'a> {
    type Item = (usize, Rc<Vec<u8>>, ribbit::Packed<Edge>);

    fn next(&mut self) -> Option<Self::Item> {
        'vertical: loop {
            // NOTE: we use `saturating_sub` to avoid underflow.
            //
            // If `self.frontier.len()` == 0, we will immediately return at `last_mut()`.
            // We can't move the len call after because `self.frontier` is mutably borrowed.
            let depth = self.frontier.len().saturating_sub(1);
            let (len, iter) = self.frontier.last_mut()?;

            'horizontal: loop {
                let Some((descend, byte, edge)) = (match iter {
                    Or::L(iter_root) => iter_root
                        .peek_mut()
                        .map(|(descend, edge)| (descend, None, edge)),
                    Or::R(iter_node) => iter_node
                        .peek_mut()
                        .map(|(descend, (key, edge))| (descend, Some(*key), edge)),
                }) else {
                    Rc::make_mut(&mut self.key).truncate(*len);
                    self.frontier.pop();
                    continue 'vertical;
                };

                let edge = edge.load_packed(Ordering::Relaxed);
                let meta = edge.meta();
                let kind = meta.kind();

                // Skip empty edges
                if kind == node::Kind::NONE {
                    iter.skip();
                    continue 'horizontal;
                }

                // Update key for current edge
                let key = Rc::make_mut(&mut self.key);

                let edge_key = meta.key().unpack();

                // Produce edge before traversing for preorder traversal
                if !mem::replace(descend, true) {
                    key.extend(byte.into_iter().chain(edge_key.bytes()));
                    return Some((depth, Rc::clone(&self.key), edge));
                }

                iter.skip();
                let len = key.len() - edge_key.len.value() as usize - byte.is_some() as usize;

                if kind == node::Kind::LEAF {
                    key.truncate(len);
                    continue 'horizontal;
                } else {
                    let node = unsafe { Edge::next_node_unchecked(edge.data(), kind) };
                    self.frontier.push((
                        len,
                        Or::R(iter::repeat(false).zip(unsafe { node.iter() }).peekable()),
                    ));
                    continue 'vertical;
                }
            }
        }
    }
}

struct ScanIter<'a> {
    window: Window<'a>,

    // root: node::EdgeIter<'a>,
    frontier: Vec<(usize, NodeIter<'a>)>,
}

impl<'a> ScanIter<'a> {
    fn new(
        low: Bound<&'a [u8]>,
        high: Bound<&'a [u8]>,
        root: &'a Atomic128<Edge>,
    ) -> Or<Option<u64>, iter::Chain<iter::Once<node::Ref<'a>>, Self>> {
        let edge = root.load_packed(Ordering::Acquire);
        let meta = edge.meta();
        let kind = meta.kind();

        let node = if kind == node::Kind::NONE {
            return Or::L(None);
        } else if kind == node::Kind::LEAF {
            return Or::L(Some(edge.data()));
        } else {
            unsafe { Edge::next_node_unchecked(edge.data(), kind) }
        };

        Or::R(iter::once(node).chain(Self {
            window: Window {
                index: 0,
                low,
                high,
                within_low: match low {
                    Bound::Unbounded => Within::Yes(0),
                    _ => Within::Maybe,
                },
                within_high: match high {
                    Bound::Unbounded => Within::Yes(0),
                    _ => Within::Maybe,
                },
            },
            // root: node::EdgeIter::new(core::slice::from_ref(root)),
            frontier: vec![(
                0,
                iter::repeat(false).zip(unsafe { node.iter() }).peekable(),
            )],
        }))
    }
}

impl<'a> Iterator for ScanIter<'a> {
    type Item = node::Ref<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        'vertical: loop {
            let (delta, iter) = self.frontier.last_mut()?;

            'horizontal: loop {
                let Some((descend, (key, edge))) = iter.peek_mut() else {
                    self.window.pop(*delta);
                    self.frontier.pop();
                    continue 'vertical;
                };

                let edge = edge.load_packed(Ordering::Relaxed);
                let meta = edge.meta();
                let kind = meta.kind();

                let node = if kind < node::Kind::NODE_3 {
                    iter.next();
                    continue 'horizontal;
                } else {
                    unsafe { Edge::next_node_unchecked(edge.data(), kind) }
                };

                if !meta
                    .key()
                    .unpack()
                    .with_bytes(Some(*key), |key| self.window.push(key))
                {
                    iter.next();
                    continue 'horizontal;
                }

                if !mem::replace(descend, true) {
                    self.frontier.push((
                        1 + edge.meta().key().len().value() as usize,
                        iter::repeat(false).zip(unsafe { node.iter() }).peekable(),
                    ));
                    return Some(node);
                } else {
                    iter.next();
                    continue 'vertical;
                }
            }
        }
    }
}

#[derive(Debug)]
struct Window<'a> {
    index: usize,
    low: Bound<&'a [u8]>,
    high: Bound<&'a [u8]>,
    within_low: Within,
    within_high: Within,
}

#[derive(Copy, Clone, Debug)]
enum Within {
    Yes(usize),
    Maybe,
}

impl<'a> Window<'a> {
    fn push(&mut self, key: &[u8]) -> bool {
        if let (Within::Yes(_), Within::Yes(_)) = (self.within_low, self.within_high) {
            self.index += key.len();
            return true;
        }

        // Check against low
        if matches!(self.within_low, Within::Maybe) {
            match self.low.map(|low| &low[self.index..]) {
                Bound::Unbounded => {
                    assert_eq!(self.index, 0);
                    self.within_low = Within::Yes(self.index);
                }
                Bound::Included(low) if key.len() == low.len() => {
                    if key < low {
                        return false;
                    }
                }
                Bound::Excluded(low) if key.len() == low.len() => {
                    if key <= low {
                        return false;
                    }
                }

                Bound::Included(low) | Bound::Excluded(low) => {
                    if key.len() < low.len() {
                        match low[..key.len()].cmp(key) {
                            cmp::Ordering::Less => self.within_low = Within::Yes(self.index),
                            cmp::Ordering::Equal => (),
                            cmp::Ordering::Greater => {
                                return false;
                            }
                        }
                    } else {
                        assert!(key.len() > low.len());
                        self.within_low = Within::Yes(self.index);
                    }
                }
            }
        }

        // Check against high
        if matches!(self.within_high, Within::Maybe) {
            match self.high.map(|high| &high[self.index..]) {
                Bound::Unbounded => {
                    assert_eq!(self.index, 0);
                    self.within_high = Within::Yes(self.index);
                }
                Bound::Included(high) if key.len() == high.len() => {
                    if key > high {
                        return false;
                    }
                }
                Bound::Excluded(high) if key.len() == high.len() => {
                    if key >= high {
                        return false;
                    }
                }
                Bound::Included(high) | Bound::Excluded(high) => {
                    if key.len() < high.len() {
                        match high[..key.len()].cmp(key) {
                            cmp::Ordering::Less => {
                                return false;
                            }
                            cmp::Ordering::Equal => (),
                            cmp::Ordering::Greater => {
                                self.within_high = Within::Yes(self.index);
                            }
                        }
                    } else {
                        assert!(key.len() > high.len());
                        return false;
                    }
                }
            }
        }

        self.index += key.len();
        true
    }

    fn pop(&mut self, delta: usize) {
        self.index -= delta;

        match self.within_low {
            Within::Yes(reset) if self.index == reset => self.within_low = Within::Maybe,
            _ => (),
        }

        match self.within_high {
            Within::Yes(reset) if self.index == reset => self.within_high = Within::Maybe,
            _ => (),
        }
    }
}
