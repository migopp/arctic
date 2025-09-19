use core::convert::Infallible;
use core::marker::PhantomData;
use core::mem;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::edge;
use crate::key;
use crate::node;
use crate::node::Node3;
use crate::Edge;
use crate::Or;

pub(crate) struct Cursor<'a, 'k, P> {
    prefix: &'k [u8],
    index: usize,
    here: &'a Atomic128<Edge>,
    history: P,
}

#[derive(Copy, Clone, Debug)]
pub(crate) enum Op {
    Node(node::Op),
    Edge(edge::Op),
}

impl<'a, 'k, P: History<'a>> Cursor<'a, 'k, P> {
    pub(crate) fn new(root: &'a Atomic128<Edge>, prefix: &'k [u8]) -> Self {
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
            let key = self.key();
            let child = unsafe { Edge::next(edge) }?;
            let len = key::Array::match_prefix(key, meta.key())?;

            match child {
                Or::R(node) => {
                    let byte = key.get(len)?;
                    let next = node.get(*byte)?;
                    self.push(len, node, next);
                    continue;
                }
                Or::L(_) => return Some(edge),
            }
        }
    }

    pub(crate) fn traverse_prefix(&mut self) -> Option<(usize, ribbit::Packed<Edge>)> {
        loop {
            let edge = self.here().load_packed(Ordering::Relaxed);
            let meta = edge.meta();
            let key = self.key();
            let child = unsafe { Edge::next(edge) }?;
            let len = key::Array::match_prefix(key, meta.key());

            // Continue traversal only if exact match
            if let (Or::R(node), Some(len)) = (child, len) {
                if let Some(next) = key.get(len).and_then(|byte| node.get(*byte)) {
                    self.push(len, node, next);
                    continue;
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
            let key = self.key();
            let child = unsafe { Edge::next(old) };
            let r#match = key::Array::match_split(key, old_meta.key());

            let (op, new) = match r#match {
                key::Match::Full(len) => match child {
                    Some(Or::R(node)) => {
                        if !matches!(self.history.freeze(), Some(freeze) if freeze == node) {
                            let byte = key[len];
                            if let Some(next) = node.get_or_reserve(byte) {
                                self.push(len, node, next);
                                continue;
                            }
                        }

                        node.freeze();
                        let (op, new) = node.replace(old_meta);
                        (Op::Node(op), new)
                    }
                    None if key.len() > key::Array::MAX => (
                        Op::Edge(edge::Op::Create),
                        Edge::new_node::<Node3, _>(key::Array::from_slice(key), None),
                    ),
                    None | Some(Or::L(_)) => (
                        Op::Edge(edge::Op::Insert),
                        Edge::new_leaf(key::Array::from_slice(key), value),
                    ),
                },
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

    #[inline]
    pub(crate) fn key(&self) -> &'k [u8] {
        &self.prefix[self.index..]
    }
}

pub(crate) trait History<'a>: Default {
    type PopError;
    fn freeze(&mut self) -> Option<node::Ref<'a>>;

    fn push(&mut self, segment: Segment<'a>);
    fn pop(&mut self) -> Result<Option<Segment<'a>>, Self::PopError>;
}

#[derive(Default)]
pub(crate) struct Optimistic<'a>(PhantomData<&'a ()>);

impl<'a> History<'a> for Optimistic<'a> {
    type PopError = ();

    #[inline(always)]
    fn freeze(&mut self) -> Option<node::Ref<'a>> {
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
    fn freeze(&mut self) -> Option<node::Ref<'a>> {
        mem::take(&mut self.freeze)
    }

    #[inline]
    fn push(&mut self, segment: Segment<'a>) {
        self.path.push(segment)
    }

    fn pop(&mut self) -> Result<Option<Segment<'a>>, Self::PopError> {
        if cfg!(feature = "validate") {
            assert!(self.freeze.is_none());
        }

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
