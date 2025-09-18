use core::convert::Infallible;
use core::marker::PhantomData;
use core::mem;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::edge;
use crate::key;
use crate::node;
use crate::node::Frozen;
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

    pub(crate) fn get(&mut self) -> Option<u64> {
        loop {
            let edge = self.here().load(Ordering::Relaxed);
            let key = self.key();

            if key::Array::from_slice_len(key, edge.meta.key.len) == edge.meta.key {
                match unsafe { edge.data.to_node(edge.meta.kind) }? {
                    Or::R(node) => {
                        let byte = key.get(edge.meta.key.len.to_usize())?;
                        let next = node.get(*byte)?;
                        self.push(edge.meta.key.len, node, next);
                        continue;
                    }
                    Or::L(leaf) => return Some(leaf),
                }
            }

            return None;
        }
    }

    pub(crate) fn traverse(&mut self) -> Option<(usize, Edge)> {
        loop {
            let edge = self.here().load(Ordering::Relaxed);
            let key = self.key();

            match unsafe { edge.data.to_node(edge.meta.kind)? } {
                // Continue traversal only if exact match
                Or::R(node)
                    if key::Array::from_slice_len(key, edge.meta.key.len) == edge.meta.key =>
                {
                    if let Some(next) = key
                        .get(edge.meta.key.len.to_usize())
                        .and_then(|byte| node.get(*byte))
                    {
                        self.push(edge.meta.key.len, node, next);
                        continue;
                    }
                }
                Or::L(_) | Or::R(_) => (),
            }

            return Some((self.index, edge));
        }
    }

    /// Return CAS operands to either insert the leaf or structurally update
    /// the tree on the way to inserting the leaf.
    pub(crate) fn traverse_or_insert(&mut self, value: u64) -> (Op, Edge, Edge) {
        loop {
            let old = self.here().load(Ordering::Relaxed);
            let key = self.key();

            let (op, new) = match old.meta.match_or_insert(key) {
                edge::Match::Full => {
                    match unsafe { old.data.to_node(old.meta.kind) } {
                        Some(Or::R(node)) => {
                            match self.history.freeze() {
                                Some(freeze) if freeze == node => (),
                                None | Some(_) => {
                                    // Must be more bytes left by no-prefix precondition
                                    let byte = key[old.meta.key.len.to_usize()];

                                    #[allow(clippy::single_match)]
                                    match node.get_or_reserve(byte) {
                                        // Fast path: no need to replace
                                        Ok(edge) => {
                                            self.push(old.meta.key.len, node, edge);
                                            continue;
                                        }
                                        Err(Frozen) => (),
                                    }
                                }
                            }

                            node.freeze();
                            let (op, new) = node.replace(&old.meta);
                            (Op::Node(op), new)
                        }
                        None if key.len() > key::Len::MAX => (
                            Op::Edge(edge::Op::Create),
                            Edge {
                                meta: edge::Meta {
                                    key: key::Array::from_slice(key),
                                    frozen: false,
                                    kind: node::Kind::Node3,
                                },
                                data: edge::Data::new_node::<Node3, _>(None),
                            },
                        ),
                        None | Some(Or::L(_)) => (
                            Op::Edge(edge::Op::Insert),
                            Edge {
                                meta: edge::Meta {
                                    key: key::Array::from_slice(key),
                                    frozen: false,
                                    kind: node::Kind::Leaf,
                                },
                                data: edge::Data::new_leaf(value),
                            },
                        ),
                    }
                }
                edge::Match::Partial { start, middle, end } => (
                    Op::Edge(edge::Op::Expand),
                    Edge {
                        meta: edge::Meta {
                            key: start,
                            frozen: false,
                            kind: node::Kind::Node3,
                        },
                        data: edge::Data::new_node::<Node3, _>(Some((
                            middle,
                            edge::Edge {
                                meta: edge::Meta {
                                    key: end,
                                    frozen: false,
                                    kind: old.meta.kind,
                                },
                                data: old.data,
                            },
                        ))),
                    },
                ),
            };

            return (op, old, new);
        }
    }

    fn push(&mut self, len: key::Len, node: node::Ref<'a>, edge: &'a Atomic128<Edge>) {
        self.index += len.to_usize();
        self.index += 1;
        self.history.push(Segment {
            len,
            edge: self.here,
            node,
        });
        self.here = edge;
    }

    pub(crate) fn pop(&mut self) -> Result<node::Ref, P::PopError> {
        let segment = self.history.pop()?.expect("Root edge can never be frozen");
        self.index -= 1;
        self.index -= segment.len.to_usize();
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
    len: key::Len,
    edge: &'a Atomic128<Edge>,
    node: node::Ref<'a>,
}
