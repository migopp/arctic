use core::convert::Infallible;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::byte;
use crate::edge;
use crate::key;
use crate::node;
use crate::node::Node3;
use crate::smr;
use crate::stat;
use crate::value::Shared;
use crate::Edge;
use crate::Op;
use crate::Value;

/// Tree traversal state.
pub(crate) struct Cursor<'g, 'l, R, V: Value, H> {
    /// SMR guard protecting allocations that overlap with `key`
    guard: smr::PathGuard<'g, 'l, V>,

    /// Total number of bits read from `key`
    bits: usize,

    /// Current key reader
    key: R,

    /// Edge this cursor currently points to
    root: &'g Atomic128<Edge<V>>,

    /// Path history of this cursor (sequence of path segments to `root`)
    history: H,
}

impl<'g, 'l, R, V, H> Cursor<'g, 'l, R, V, H>
where
    R: key::Read,
    V: Value,
    H: History<'g, R, V>,
{
    #[inline]
    pub(crate) fn new(
        smr: &'l mut smr::Local<'g, V>,
        root: &'g Atomic128<Edge<V>>,
        key: R,
    ) -> Self {
        Self {
            guard: smr.guard(key.peek_all()),
            bits: 0,
            root,
            key,
            history: H::new(root, key),
        }
    }

    #[inline]
    pub(crate) fn root(&self) -> &'g Atomic128<Edge<V>> {
        self.root
    }

    #[inline]
    pub(crate) fn bits(&self) -> usize {
        self.bits
    }

    #[inline]
    pub(crate) unsafe fn retire(&mut self, edge: ribbit::Packed<Edge<V>>) {
        unsafe { self.guard.retire(edge) }
    }

    #[inline]
    pub(crate) fn into_guard(self) -> smr::PathGuard<'g, 'l, V> {
        self.guard
    }

    #[inline]
    pub(crate) fn traverse_exact(&mut self) -> Option<Result<ribbit::Packed<Edge<V>>, ()>> {
        loop {
            let mut edge = self.root().load_packed(Ordering::Relaxed);

            if edge.is_scan() {
                match self.wait_for_scan(stat::Counter::ScanUpdate) {
                    Ok(safe) => edge = safe,
                    Err(()) => return Some(Err(())),
                }
            }

            let meta = edge.meta();

            let save = self.key;
            let len = meta.key().match_exact(&mut self.key)?;

            // Fast path: traversal
            if edge.is_node() {
                let byte = self.key.next()?;
                let node = unsafe { edge.data().into_node_unchecked() };
                let next = node.get(byte)?;
                self.push(save, len, node, next);
                continue;
            }

            self.key = save;

            return if meta.frozen() {
                Some(Err(()))
            } else if meta.leaf() {
                Some(Ok(edge))
            } else {
                validate!(edge.data().is_null());
                None
            };
        }
    }

    /// Return CAS operands to either insert the leaf or structurally update
    /// the tree on the way to inserting the leaf.
    #[inline]
    pub(crate) fn traverse_or_insert(
        &mut self,
        leaf: ribbit::Packed<Edge<V>>,
    ) -> Result<(Op, ribbit::Packed<Edge<V>>, ribbit::Packed<Edge<V>>), ()> {
        loop {
            let mut old = self.root().load_packed(Ordering::Relaxed);

            if old.is_scan() {
                old = self.wait_for_scan(stat::Counter::ScanInsert)?;
            }

            let old_meta = old.meta();
            let old_data = old.data();
            let save = self.key;
            let r#match = old_meta.key().match_split(&mut self.key);

            // Fast path: traverse
            if let byte::MatchSplit::Full(len) = r#match {
                if old.is_node() {
                    let byte = self.key.next().unwrap();
                    let node = unsafe { old_data.into_node_unchecked() };
                    if let Some(next) = node.get_or_reserve(byte) {
                        self.push(save, len, node, next);
                        continue;
                    }
                }
            }

            if old_meta.frozen() {
                return Err(());
            }

            // Revert key to before the current edge
            self.key = save;

            let (op, new) = match r#match {
                byte::MatchSplit::Full(_) if old.is_node() => {
                    let node = unsafe { old_data.into_node_unchecked() };
                    let (op, new) = node.replace(old_meta);
                    (Op::Node(op), new)
                }
                byte::MatchSplit::Full(_) if self.key.bits() > byte::Len::MAX.bits() as usize => (
                    Op::Edge(edge::Op::Create),
                    Edge::new_node::<Node3<V>, _>(self.key.peek(byte::Len::MAX), None),
                ),
                byte::MatchSplit::Full(_) => {
                    (
                        Op::Edge(edge::Op::Insert),
                        leaf.with_meta(edge::Meta::LEAF.with_key(self.key.peek(unsafe {
                            byte::Len::from_bits_unchecked(self.key.bits() as u8)
                        }))),
                    )
                }
                byte::MatchSplit::Partial { start, middle, end } => (
                    Op::Edge(edge::Op::Expand),
                    Edge::new_node::<Node3<V>, _>(
                        start,
                        Some((middle, old.with_meta(old_meta.with_key(end)))),
                    ),
                ),
            };

            return Ok((op, old, new));
        }
    }

    #[cold]
    pub(crate) fn wait_for_scan(
        &self,
        counter: stat::Counter,
    ) -> Result<ribbit::Packed<Edge<V>>, ()> {
        stat::increment(counter);

        loop {
            core::hint::spin_loop();
            let edge = self.root().load_packed(Ordering::Acquire);

            if !edge.is_scan() {
                return Ok(edge);
            }

            if edge.meta().frozen() {
                return Err(());
            }
        }
    }

    #[inline]
    fn push(
        &mut self,
        key: R,
        len: byte::Len,
        node: node::Ref<'g, V>,
        edge: &'g Atomic128<Edge<V>>,
    ) {
        // 1 extra byte for node
        self.bits += 8 + len.bits() as usize;
        self.history.push(Segment {
            key,
            len,
            edge: core::mem::replace(&mut self.root, edge),
            node,
        })
    }

    #[cold]
    pub(crate) fn pop(&mut self) -> Result<node::Ref<'g, V>, H::PopError> {
        let segment = self.history.pop()?.expect("Root edge can never be frozen");
        self.bits -= segment.len.bits() as usize + 8;
        self.key = segment.key;
        self.root = segment.edge;
        Ok(segment.node)
    }
}

impl<'g, 'l, R: key::Read, V: Value> Cursor<'g, 'l, R, V, Optimistic> {
    #[inline]
    pub(crate) fn traverse_value(mut self) -> Option<Shared<'g, 'l, V>> {
        loop {
            let edge = self.root.load_packed(Ordering::Relaxed);
            let meta = edge.meta();

            let _ = meta.key().match_exact(&mut self.key)?;
            let data = edge.data();

            if meta.leaf() {
                return Some(unsafe { V::guard(self.guard, data.into_leaf()) });
            } else if data.is_null() {
                return None;
            } else {
                let byte = self.key.next()?;
                let data = edge.data();
                let node = unsafe { data.into_node_unchecked() };
                self.root = node.get(byte)?;
            }
        }
    }
}

pub(crate) struct Prefix<'g, 'l, R, V: Value, H> {
    prefix: R,
    cursor: Cursor<'g, 'l, R, V, H>,
}

impl<'g, 'l, R, V, H> Prefix<'g, 'l, R, V, H>
where
    R: key::Read,
    V: Value,
    H: History<'g, R, V>,
{
    pub(crate) fn new_prefix(
        smr: &'l mut smr::Local<'g, V>,
        root: &'g Atomic128<Edge<V>>,
        prefix: R,
    ) -> Option<Self> {
        let mut cursor = Self {
            prefix,
            cursor: Cursor::new(smr, root, prefix),
        };
        cursor.traverse_prefix()?;
        Some(cursor)
    }

    pub(crate) fn new_range(
        smr: &'l mut smr::Local<'g, V>,
        root: &'g Atomic128<Edge<V>>,
        min: R,
        max: R,
    ) -> Option<Self> {
        let prefix = min.prefix(&max);
        Self::new_prefix(smr, root, prefix)
    }

    pub(crate) fn prefix(&self) -> R {
        self.prefix.slice(self.cursor.bits())
    }

    #[inline]
    pub(crate) fn into_guard(self) -> smr::PathGuard<'g, 'l, V> {
        self.cursor.guard
    }

    pub(crate) fn traverse_prefix(&mut self) -> Option<ribbit::Packed<Edge<V>>> {
        let (key, edge) = loop {
            let edge = self.cursor.root.load_packed(Ordering::Acquire);
            let meta = edge.meta();
            let data = edge.data();
            let save = self.cursor.key;

            match meta.key().match_prefix(&mut self.cursor.key)? {
                byte::MatchPrefix::Full(len) if edge.is_node() => {
                    let node = unsafe { data.into_node_unchecked() };
                    let Some(byte) = self.cursor.key.next() else {
                        break (save, edge);
                    };
                    let next = node.get(byte)?;
                    self.cursor.push(save, len, node, next);
                }
                byte::MatchPrefix::Full(_) | byte::MatchPrefix::Partial => match edge.is_null() {
                    true => return None,
                    false => break (save, edge),
                },
            }
        };

        self.cursor.key = key;
        Some(edge)
    }
}

impl<'g, 'l, R: key::Read, V: Value> Cursor<'g, 'l, R, V, Hybrid<'g, R, V>> {
    pub(crate) fn upgrade(&mut self) {
        let (root, key) = match self.history {
            Hybrid::Optimistic { key, root } => (root, key),
            Hybrid::Pessimistic { .. } => return,
        };

        self.root = root;
        self.key = key;
        self.bits = 0;
        self.history = Hybrid::Pessimistic {
            key,
            pessimistic: Pessimistic { path: Vec::new() },
        }
    }
}

impl<'g, 'l, R, V: Value, H> core::ops::Deref for Prefix<'g, 'l, R, V, H> {
    type Target = Cursor<'g, 'l, R, V, H>;
    fn deref(&self) -> &Self::Target {
        &self.cursor
    }
}

impl<'g, 'l, R, V: Value, H> core::ops::DerefMut for Prefix<'g, 'l, R, V, H> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.cursor
    }
}

pub(crate) trait History<'g, R, V> {
    type PopError;

    fn new(root: &'g Atomic128<Edge<V>>, key: R) -> Self;

    fn push(&mut self, segment: Segment<'g, R, V>);
    fn pop(&mut self) -> Result<Option<Segment<'g, R, V>>, Self::PopError>;
}

pub(crate) struct Optimistic;

impl<'g, R, V> History<'g, R, V> for Optimistic {
    type PopError = ();

    fn new(_root: &'g Atomic128<Edge<V>>, _key: R) -> Self {
        Self
    }

    #[inline]
    fn push(&mut self, _segment: Segment<'g, R, V>) {}

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'g, R, V>>, Self::PopError> {
        Err(())
    }
}

pub(crate) struct Pessimistic<'g, R, V> {
    path: Vec<Segment<'g, R, V>>,
}

impl<'g, R, V> History<'g, R, V> for Pessimistic<'g, R, V> {
    type PopError = Infallible;

    fn new(_root: &'g Atomic128<Edge<V>>, _key: R) -> Self {
        Self { path: Vec::new() }
    }

    #[inline]
    fn push(&mut self, segment: Segment<'g, R, V>) {
        self.path.push(segment);
    }

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'g, R, V>>, Self::PopError> {
        Ok(self.path.pop())
    }
}

pub(crate) enum Hybrid<'g, R, V> {
    Optimistic {
        key: R,
        root: &'g Atomic128<Edge<V>>,
    },
    Pessimistic {
        key: R,
        pessimistic: Pessimistic<'g, R, V>,
    },
}

impl<'g, R: Copy, V> History<'g, R, V> for Hybrid<'g, R, V> {
    type PopError = ();

    fn new(root: &'g Atomic128<Edge<V>>, key: R) -> Self {
        Self::Optimistic { key, root }
    }

    #[inline]
    fn push(&mut self, segment: Segment<'g, R, V>) {
        match self {
            Hybrid::Optimistic { .. } => (),
            Hybrid::Pessimistic { pessimistic, .. } => pessimistic.push(segment),
        }
    }

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'g, R, V>>, Self::PopError> {
        match self {
            Hybrid::Optimistic { .. } => Err(()),
            Hybrid::Pessimistic { pessimistic, .. } => Ok(pessimistic.pop().unwrap()),
        }
    }
}

/// A path along the tree is composed of 0 or more path segments.
pub(crate) struct Segment<'g, R, V> {
    /// Edge to match
    edge: &'g Atomic128<Edge<V>>,

    /// Key before matching on `edge`
    key: R,

    /// Number of bytes matched along `edge`
    len: byte::Len,

    /// Node underneath `edge`
    node: node::Ref<'g, V>,
}
