use core::convert::Infallible;
use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;
use ribbit::u3;

use crate::byte;
use crate::edge;
use crate::key;
use crate::node;
use crate::node::Node3;
use crate::raw::Op;
use crate::Edge;

/// Stateful traversal over tree.
pub(crate) struct Cursor<'a, K, H> {
    prefix: ribbit::Packed<byte::Array>,
    index: usize,
    key: K,
    root: &'a Atomic128<Edge>,
    history: H,
}

impl<'a, K: key::Iterator, H: History<'a, K>> Cursor<'a, K, H> {
    #[inline]
    pub(crate) fn new(key: K, root: &'a Atomic128<Edge>) -> Self {
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
            let kind = meta.kind();

            // Fast path: traversal
            if kind >= node::Kind::NODE_3 {
                let Some(byte) = self.key.next() else {
                    return Ok(None);
                };
                let data = edge.data();
                let node = unsafe { Edge::next_node_unchecked(data, kind) };
                let Some(next) = node.get(byte) else {
                    return Ok(None);
                };
                self.step(save, len, node, next);
                continue;
            }

            // Prepare to CAS
            return if meta.frozen() {
                Err(())
            } else if kind == node::Kind::LEAF {
                Ok(Some(edge))
            } else {
                validate_eq!(kind, node::Kind::NONE);
                Ok(None)
            };
        }
    }

    #[inline]
    pub(crate) fn traverse_prefix(&mut self) -> usize {
        loop {
            let edge = self.root().load_packed(Ordering::Relaxed);
            let meta = edge.meta();
            let kind = meta.kind();
            let save = self.key.clone();

            // Continue traversal only if exact match
            if kind >= node::Kind::NODE_3 {
                if let Some(len) = meta.key().match_prefix(&mut self.key) {
                    let node = unsafe { Edge::next_node_unchecked(edge.data(), kind) };
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
            let save = self.key.clone();
            let r#match = old_meta.key().match_split(&mut self.key);
            let kind = old_meta.kind();

            // Fast path: traverse
            if let byte::Match::Full(len) = r#match {
                if kind >= node::Kind::NODE_3 {
                    let byte = self.key.next().unwrap();
                    let node = unsafe { Edge::next_node_unchecked(old.data(), kind) };
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
                byte::Match::Full(_) => {
                    if kind >= node::Kind::NODE_3 {
                        let node = unsafe { Edge::next_node_unchecked(old.data(), kind) };
                        let (op, new) = node.replace(old_meta);
                        (Op::Node(op), new)
                    } else if kind == node::Kind::NONE
                        && self.key.len() > byte::Array::MAX_LEN.value() as usize
                    {
                        (
                            Op::Edge(edge::Op::Create),
                            Edge::new_node::<Node3, _>(self.key.peek_all(), None),
                        )
                    } else {
                        (
                            Op::Edge(edge::Op::Insert),
                            Edge::new_leaf(self.key.peek_all(), value),
                        )
                    }
                }
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
    fn step(&mut self, key: K, len: u3, node: node::Ref<'a>, edge: &'a Atomic128<Edge>) {
        // 1 extra byte for node
        self.index += len.value() as usize + 1;
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
        self.index -= segment.len.value() as usize + 1;
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

impl<K> Default for Optimistic<K> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<'a, K> History<'a, K> for Optimistic<K> {
    type PopError = ();

    #[inline]
    fn push(&mut self, _segment: Segment<'a, K>) {}

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'a, K>>, Self::PopError> {
        Err(())
    }
}

pub(crate) struct Pessimistic<'a, K> {
    path: Vec<Segment<'a, K>>,
}

impl<K> Default for Pessimistic<'_, K> {
    fn default() -> Self {
        Self {
            path: Vec::default(),
        }
    }
}

impl<'a, K: Clone> History<'a, K> for Pessimistic<'a, K> {
    type PopError = Infallible;

    #[inline]
    fn push(&mut self, segment: Segment<'a, K>) {
        self.path.push(segment);
    }

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'a, K>>, Self::PopError> {
        Ok(self.path.pop())
    }
}

/// Path segment consists of:
/// - Current key before matching on edge
/// - Number of bytes matched along edge
/// - Edge to match next
/// - Node underneath edge
#[derive(Debug)]
pub(crate) struct Segment<'a, K> {
    key: K,
    len: u3,
    edge: &'a Atomic128<Edge>,
    node: node::Ref<'a>,
}
