use core::convert::Infallible;
use core::marker::PhantomData;
use core::mem;
use core::num::NonZeroU64;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::edge;
use crate::key;
use crate::node;
use crate::node::Node3;
use crate::Edge;
use crate::Key;

pub(crate) struct Cursor<'a, 'k, K: ?Sized, P> {
    prefix: &'k K,
    index: usize,
    here: &'a Atomic128<Edge>,
    history: P,
}

#[derive(Copy, Clone, Debug)]
pub(crate) enum Op {
    Node(node::Op),
    Edge(edge::Op),
}

impl<'a, 'k, K: Key + ?Sized, P: History<'a>> Cursor<'a, 'k, K, P> {
    pub(crate) fn new(root: &'a Atomic128<Edge>, prefix: &'k K) -> Self {
        Self {
            prefix,
            index: 0,
            here: root,
            history: P::default(),
        }
    }

    pub(crate) fn traverse_exact(&mut self) -> Option<ribbit::Packed<Edge>> {
        loop {
            let edge = self.here().load_packed(Ordering::Relaxed);
            let meta = edge.meta();
            let len = key::Array::match_prefix(self.prefix, self.index, meta.key())?;

            let kind = meta.kind();
            if kind >= node::Kind::NODE_3 {
                let byte = self.prefix.get(self.index + len)?;
                let data = edge.data();
                let node = unsafe { Edge::next_node_unchecked(data, kind) };
                let next = node.get(byte)?;
                self.push(len, node, next);
                continue;
            } else if kind == node::Kind::LEAF {
                return Some(edge);
            } else {
                validate_eq!(kind, node::Kind::NONE);
                return None;
            }
        }
    }

    pub(crate) fn traverse_prefix(&mut self) -> Option<(usize, ribbit::Packed<Edge>)> {
        loop {
            let edge = self.here().load_packed(Ordering::Relaxed);
            let meta = edge.meta();

            let kind = meta.kind();

            // Continue traversal only if exact match
            if kind >= node::Kind::NODE_3 {
                if let Some(len) = key::Array::match_prefix(self.prefix, self.index, meta.key()) {
                    let node = unsafe { Edge::next_node_unchecked(edge.data(), kind) };
                    if let Some(next) = self
                        .prefix
                        .get(self.index + len)
                        .and_then(|byte| node.get(byte))
                    {
                        self.push(len, node, next);
                        continue;
                    }
                }
            }

            return Some((self.index, edge));
        }
    }

    /// Return CAS operands to either insert the leaf or structurally update
    /// the tree on the way to inserting the leaf.
    pub(crate) fn traverse_or_insert(
        &mut self,
        value: u64,
    ) -> (Op, ribbit::Packed<Edge>, ribbit::Packed<Edge>) {
        loop {
            let old = self.here().load_packed(Ordering::Relaxed);
            let old_meta = old.meta();
            let r#match = key::Array::match_split(self.prefix, self.index, old_meta.key());

            let (op, new) = match r#match {
                key::Match::Full(len) => {
                    let kind = old_meta.kind();

                    if kind >= node::Kind::NODE_3 {
                        let old_data = old.data();
                        let node = unsafe { Edge::next_node_unchecked(old_data, kind) };
                        if !matches!(self.history.freeze(), Some(freeze) if freeze.get() == old_data)
                        {
                            let byte = self.prefix.get(self.index + len).unwrap();
                            if let Some(next) = node.get_or_reserve(byte) {
                                self.push(len, node, next);
                                continue;
                            }
                        }

                        node.freeze();
                        let (op, new) = node.replace(old_meta);
                        (Op::Node(op), new)
                    } else if kind == node::Kind::NONE
                        && self.prefix.len() - self.index > key::Array::MAX
                    {
                        (
                            Op::Edge(edge::Op::Create),
                            Edge::new_node::<Node3, _>(
                                key::Array::from_slice(self.prefix, self.index),
                                None,
                            ),
                        )
                    } else {
                        (
                            Op::Edge(edge::Op::Insert),
                            Edge::new_leaf(key::Array::from_slice(self.prefix, self.index), value),
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

            return (op, old, new);
        }
    }

    fn push(&mut self, len: usize, node: node::Ref<'a>, next: &'a Atomic128<Edge>) {
        self.index += len + 1;
        self.history.push(Segment {
            len,
            edge: self.here,
            node,
        });
        self.here = next;
    }

    pub(crate) fn pop(&mut self) -> Result<node::Ref, P::PopError> {
        let segment = self.history.pop()?.expect("Root edge can never be frozen");
        self.index -= segment.len + 1;
        self.here = segment.edge;
        Ok(segment.node)
    }

    #[inline]
    pub(crate) fn here(&self) -> &Atomic128<Edge> {
        self.here
    }
}

pub(crate) trait History<'a>: Default {
    type PopError;
    fn freeze(&mut self) -> Option<NonZeroU64>;

    fn push(&mut self, segment: Segment<'a>);
    fn pop(&mut self) -> Result<Option<Segment<'a>>, Self::PopError>;
}

#[derive(Default)]
pub(crate) struct Optimistic<'a>(PhantomData<&'a ()>);

impl<'a> History<'a> for Optimistic<'a> {
    type PopError = ();

    #[inline(always)]
    fn freeze(&mut self) -> Option<NonZeroU64> {
        None
    }

    #[inline(always)]
    fn push(&mut self, _segment: Segment<'a>) {}

    #[inline(always)]
    fn pop(&mut self) -> Result<Option<Segment<'a>>, Self::PopError> {
        Err(())
    }
}

#[derive(Default)]
pub(crate) struct Pessimistic<'a> {
    freeze: Option<node::Ref<'a>>,
    path: Vec<Segment<'a>>,
}

impl<'a> History<'a> for Pessimistic<'a> {
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
    fn push(&mut self, segment: Segment<'a>) {
        self.path.push(segment)
    }

    fn pop(&mut self) -> Result<Option<Segment<'a>>, Self::PopError> {
        validate!(self.freeze.is_none());
        self.freeze = self.path.last().map(|segment| segment.node);
        Ok(self.path.pop())
    }
}

#[derive(Debug)]
pub(crate) struct Segment<'a> {
    len: usize,
    edge: &'a Atomic128<Edge>,
    node: node::Ref<'a>,
}
