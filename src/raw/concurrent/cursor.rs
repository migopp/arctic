use core::convert::Infallible;
use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::byte;
use crate::edge;
use crate::key;
use crate::node;
use crate::node::Node3;
use crate::raw::Op;
use crate::Edge;

/// Stateful traversal over tree.
pub(crate) struct Cursor<'a, R, H> {
    prefix: ribbit::Packed<byte::Array>,
    index: usize,
    key: R,
    root: &'a Atomic128<Edge>,
    history: H,
}

impl<'a, R: key::Read, H: History<'a, R>> Cursor<'a, R, H> {
    #[inline]
    pub(crate) fn new(key: R, root: &'a Atomic128<Edge>) -> Self {
        Self {
            prefix: key.peek_all(),
            index: 0,
            key: key.clone(),
            root,
            history: H::default(),
        }
    }

    #[inline]
    pub(crate) fn root(&self) -> &'a Atomic128<Edge> {
        self.root
    }

    #[inline]
    pub(crate) fn prefix(&self) -> ribbit::Packed<byte::Array> {
        self.prefix.slice(self.index)
    }

    #[inline]
    pub(crate) fn traverse_exact(&mut self) -> Result<Option<ribbit::Packed<Edge>>, ()> {
        loop {
            let edge = self.root().load_packed(Ordering::Relaxed);
            let meta = edge.meta();
            let save = self.key.clone();
            let Some(len) = meta.key().match_prefix(&mut self.key) else {
                return Ok(None);
            };

            let data = edge.data();

            // Fast path: traversal
            if !meta.leaf() && data != 0 {
                let Some(byte) = self.key.next() else {
                    return Ok(None);
                };
                let node = unsafe { Edge::next_node_unchecked(data) };
                let Some(next) = node.get(byte) else {
                    return Ok(None);
                };
                self.step(save, len, node, next);
                continue;
            }

            self.key = save;

            // Prepare to CAS
            return if meta.frozen() {
                Err(())
            } else if meta.leaf() {
                Ok(Some(edge))
            } else {
                validate_eq!(data, 0);
                Ok(None)
            };
        }
    }

    #[inline]
    pub(crate) fn traverse_prefix(&mut self) -> usize {
        loop {
            let edge = self.root().load_packed(Ordering::Relaxed);
            let meta = edge.meta();
            let data = edge.data();
            let save = self.key.clone();

            // Continue traversal only if exact match
            if !meta.leaf() && data != 0 {
                if let Some(len) = meta.key().match_prefix(&mut self.key) {
                    let node = unsafe { Edge::next_node_unchecked(data) };
                    if let Some(next) = self.key.next().and_then(|byte| node.get(byte)) {
                        self.step(save, len, node, next);
                        continue;
                    }
                }
            }

            self.key = save;
            return self.index;
        }
    }

    /// Return CAS operands to either insert the leaf or structurally update
    /// the tree on the way to inserting the leaf.
    #[inline]
    pub(crate) fn traverse_or_insert(
        &mut self,
        value: u64,
    ) -> Result<(Op, ribbit::Packed<Edge>, ribbit::Packed<Edge>), ()> {
        loop {
            let old = self.root().load_packed(Ordering::Relaxed);
            let old_meta = old.meta();
            let old_data = old.data();
            let save = self.key.clone();
            let r#match = old_meta.key().match_split(&mut self.key);

            // Fast path: traverse
            if let byte::Match::Full(len) = r#match {
                if !old_meta.leaf() && old_data > 0 {
                    let byte = self.key.next().unwrap();
                    let node = unsafe { Edge::next_node_unchecked(old_data) };
                    if let Some(next) = node.get_or_reserve(byte) {
                        self.step(save, len, node, next);
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
                byte::Match::Full(_) if !old_meta.leaf() && old_data > 0 => {
                    let node = unsafe { Edge::next_node_unchecked(old_data) };
                    let (op, new) = node.replace(old_meta);
                    (Op::Node(op), new)
                }
                byte::Match::Full(_) if self.key.len() > byte::Array::MAX_LEN.value() as usize => (
                    Op::Edge(edge::Op::Create),
                    Edge::new_node::<Node3, _>(self.key.peek_all(), None),
                ),
                byte::Match::Full(_) => (
                    Op::Edge(edge::Op::Insert),
                    Edge::new_leaf(self.key.peek_all(), value),
                ),
                byte::Match::Partial { start, middle, end } => (
                    Op::Edge(edge::Op::Expand),
                    Edge::new_node::<Node3, _>(
                        start,
                        Some((
                            middle,
                            old.with_meta(old.meta().with_key(end).with_frozen(false)),
                        )),
                    ),
                ),
            };

            return Ok((op, old, new));
        }
    }

    #[inline]
    fn step(
        &mut self,
        key: R,
        len: ribbit::Packed<byte::Len>,
        node: node::Ref<'a>,
        edge: &'a Atomic128<Edge>,
    ) {
        // 1 extra byte for node
        self.index += len.value() as usize + 8;
        self.history.push(Segment {
            key,
            len,
            edge: core::mem::replace(&mut self.root, edge),
            node,
        })
    }

    #[cold]
    pub(crate) fn pop(&mut self) -> Result<node::Ref<'a>, H::PopError> {
        let segment = self.history.pop()?.expect("Root edge can never be frozen");
        self.index -= segment.len.value() as usize + 8;
        self.key = segment.key;
        self.root = segment.edge;
        Ok(segment.node)
    }
}

pub(crate) trait History<'a, K>: Default {
    type PopError;

    fn push(&mut self, segment: Segment<'a, K>);
    fn pop(&mut self) -> Result<Option<Segment<'a, K>>, Self::PopError>;
}

pub(crate) struct Optimistic<K>(PhantomData<K>);

impl<R> Default for Optimistic<R> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<'a, R> History<'a, R> for Optimistic<R> {
    type PopError = ();

    #[inline]
    fn push(&mut self, _segment: Segment<'a, R>) {}

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'a, R>>, Self::PopError> {
        Err(())
    }
}

pub(crate) struct Pessimistic<'a, R> {
    path: Vec<Segment<'a, R>>,
}

impl<R> Default for Pessimistic<'_, R> {
    fn default() -> Self {
        Self {
            path: Vec::default(),
        }
    }
}

impl<'a, R> History<'a, R> for Pessimistic<'a, R> {
    type PopError = Infallible;

    #[inline]
    fn push(&mut self, segment: Segment<'a, R>) {
        self.path.push(segment);
    }

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'a, R>>, Self::PopError> {
        Ok(self.path.pop())
    }
}

/// Path segment consists of:
/// - Current key before matching on edge
/// - Number of bytes matched along edge
/// - Edge to match next
/// - Node underneath edge
#[derive(Debug)]
pub(crate) struct Segment<'a, R> {
    key: R,
    len: ribbit::Packed<byte::Len>,
    edge: &'a Atomic128<Edge>,
    node: node::Ref<'a>,
}
