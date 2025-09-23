use core::convert::Infallible;
use core::marker::PhantomData;
use core::mem;
use core::num::NonZeroU64;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;
use ribbit::u3;

use crate::edge;
use crate::key;
use crate::node;
use crate::node::Node3;
use crate::Edge;

/// Stateful traversal over tree.
pub(crate) struct Cursor<'a, K, P> {
    key: K,
    index: usize,
    root: &'a Atomic128<Edge>,
    history: P,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    Node(node::Op),
    Edge(edge::Op),
}

impl<'a, K: key::Iterator, P: History<'a, K>> Cursor<'a, K, P> {
    #[inline]
    pub(crate) fn new(key: K, root: &'a Atomic128<Edge>) -> Self {
        Self {
            key: key.clone(),
            index: 0,
            root,
            history: P::default(),
        }
    }

    #[inline]
    pub(crate) fn traverse_exact(&mut self) -> Option<ribbit::Packed<Edge>> {
        loop {
            let edge = self.root().load_packed(Ordering::Relaxed);
            let meta = edge.meta();
            let save = self.key.clone();
            let len = key::Array::match_prefix(&mut self.key, meta.key())?;

            let kind = meta.kind();
            if kind >= node::Kind::NODE_3 {
                let byte = self.key.next()?;
                let data = edge.data();
                let node = unsafe { Edge::next_node_unchecked(data, kind) };
                let next = node.get(byte)?;
                self.step(save, len, node, next);
                continue;
            } else if kind == node::Kind::LEAF {
                return Some(edge);
            } else {
                validate_eq!(kind, node::Kind::NONE);
                return None;
            }
        }
    }

    #[inline]
    #[expect(dead_code)]
    pub(crate) fn traverse_prefix(&mut self) -> Option<ribbit::Packed<Edge>> {
        loop {
            let edge = self.root().load_packed(Ordering::Relaxed);
            let meta = edge.meta();
            let kind = meta.kind();
            let save = self.key.clone();

            // Continue traversal only if exact match
            if kind >= node::Kind::NODE_3 {
                if let Some(len) = key::Array::match_prefix(&mut self.key, meta.key()) {
                    let node = unsafe { Edge::next_node_unchecked(edge.data(), kind) };
                    if let Some(next) = self.key.next().and_then(|byte| node.get(byte)) {
                        self.step(save, len, node, next);
                        continue;
                    }
                }
            }

            self.key = save;
            return Some(edge);
        }
    }

    /// Return CAS operands to either insert the leaf or structurally update
    /// the tree on the way to inserting the leaf.
    #[inline]
    pub(crate) fn traverse_or_insert(
        &mut self,
        value: u64,
    ) -> (Op, ribbit::Packed<Edge>, ribbit::Packed<Edge>) {
        loop {
            let old = self.root().load_packed(Ordering::Relaxed);
            let old_meta = old.meta();
            let save = self.key.clone();
            let r#match = key::Array::match_split(&mut self.key, old_meta.key());

            let (op, new) = match r#match {
                key::Match::Full(len) => {
                    let kind = old_meta.kind();

                    if kind >= node::Kind::NODE_3 {
                        let old_data = old.data();
                        let node = unsafe { Edge::next_node_unchecked(old_data, kind) };
                        if !matches!(self.history.freeze(), Some(freeze) if freeze.get() == old_data)
                        {
                            let byte = self.key.next().unwrap();
                            if let Some(next) = node.get_or_reserve(byte) {
                                self.step(save, len, node, next);
                                continue;
                            }

                            crate::cold();
                        }

                        node.freeze();
                        let (op, new) = node.replace(old_meta);
                        (Op::Node(op), new)
                    } else if kind == node::Kind::NONE
                        && save.len() > key::Array::MAX_LEN.value() as usize
                    {
                        (
                            Op::Edge(edge::Op::Create),
                            Edge::new_node::<Node3, _>(key::Array::from_slice(save.clone()), None),
                        )
                    } else {
                        (
                            Op::Edge(edge::Op::Insert),
                            Edge::new_leaf(key::Array::from_slice(save.clone()), value),
                        )
                    }
                }
                key::Match::Partial { start, middle, end } => (
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

            // Revert key to before the current edge
            self.key = save;
            return (op, old, new);
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
    pub(crate) fn pop(&mut self) -> Result<node::Ref<'a>, P::PopError> {
        let segment = self.history.pop()?.expect("Root edge can never be frozen");
        self.index -= segment.len.value() as usize + 1;
        self.key = segment.key;
        self.root = segment.edge;
        Ok(segment.node)
    }

    #[inline]
    pub(crate) fn root(&self) -> &'a Atomic128<Edge> {
        self.root
    }

    #[inline]
    pub(crate) fn index(&self) -> usize {
        self.index
    }
}

pub(crate) trait History<'a, K>: Default {
    type PopError;

    fn freeze(&mut self) -> Option<NonZeroU64>;
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
    fn freeze(&mut self) -> Option<NonZeroU64> {
        None
    }

    #[inline]
    fn push(&mut self, _segment: Segment<'a, K>) {}

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'a, K>>, Self::PopError> {
        Err(())
    }
}

pub(crate) struct Pessimistic<'a, K> {
    freeze: Option<node::Ref<'a>>,
    path: Vec<Segment<'a, K>>,
}

impl<K> Default for Pessimistic<'_, K> {
    fn default() -> Self {
        Self {
            freeze: None,
            path: Vec::default(),
        }
    }
}

impl<'a, K: Clone> History<'a, K> for Pessimistic<'a, K> {
    type PopError = Infallible;

    #[inline]
    fn freeze(&mut self) -> Option<NonZeroU64> {
        Some(match mem::take(&mut self.freeze)? {
            node::Ref::Node3(node) => unsafe { NonZeroU64::new_unchecked(node as *const _ as u64) },
            node::Ref::Node15(node) => unsafe {
                NonZeroU64::new_unchecked(node as *const _ as u64)
            },
            node::Ref::Node256(node) => unsafe {
                NonZeroU64::new_unchecked(node as *const _ as u64)
            },
        })
    }

    #[inline]
    fn push(&mut self, segment: Segment<'a, K>) {
        self.path.push(segment);
    }

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'a, K>>, Self::PopError> {
        validate!(self.freeze.is_none());
        self.freeze = self.path.last().map(|segment| segment.node);
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
