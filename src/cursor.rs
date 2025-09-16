use core::convert::Infallible;
use core::marker::PhantomData;
use core::mem;
use core::sync::atomic::Ordering;

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
    here: &'a Edge,
    history: P,
}

#[derive(Debug)]
pub(crate) enum Op {
    Node(node::Op),
    Edge(edge::Op),
}

impl<'a, 'k, P: History<'a>> Cursor<'a, 'k, P> {
    pub(crate) fn new(root: &'a Edge, prefix: &'k [u8]) -> Self {
        Self {
            prefix,
            index: 0,
            here: root,
            history: P::default(),
        }
    }

    pub(crate) fn get(&mut self) -> Option<u64> {
        loop {
            let (meta, data) = self.here().load(Ordering::Relaxed);
            let key = self.key();

            let node = match unsafe { data.to_node(meta.kind) }? {
                // Stop unconditionally at a leaf due to precondition
                Or::L(leaf) => return Some(leaf),

                // Continue traversal only if exact match
                Or::R(node) if key::Array::from_slice_len(key, meta.key.len) == meta.key => node,
                Or::R(_) => return None,
            };

            let byte = key.get(meta.key.len.to_usize())?;
            let next = node.get(*byte)?;
            self.push(meta.key.len, node, next);
        }
    }

    pub(crate) fn traverse(&mut self) -> Option<(usize, edge::Meta, edge::Data)> {
        loop {
            let (meta, data) = self.here().load(Ordering::Relaxed);
            let key = self.key();

            match unsafe { data.to_node(meta.kind)? } {
                // Continue traversal only if exact match
                Or::R(node) if key::Array::from_slice_len(key, meta.key.len) == meta.key => {
                    if let Some(next) = key
                        .get(meta.key.len.to_usize())
                        .and_then(|byte| node.get(*byte))
                    {
                        self.push(meta.key.len, node, next);
                        continue;
                    }
                }
                Or::L(_) | Or::R(_) => (),
            }

            return Some((self.index, meta, data));
        }
    }

    /// Return CAS operands to either insert the leaf or structurally update
    /// the tree on the way to inserting the leaf.
    pub(crate) fn traverse_or_insert(
        &mut self,
        value: u64,
    ) -> (Op, (edge::Meta, edge::Data), (edge::Meta, edge::Data)) {
        loop {
            let (old_meta, old_data) = self.here().load(Ordering::Relaxed);
            let key = self.key();

            match old_meta.match_or_insert(key) {
                edge::Match::Full => (),
                edge::Match::Partial { start, middle, end } => {
                    return (
                        Op::Edge(edge::Op::Expand),
                        (old_meta, old_data),
                        (
                            edge::Meta {
                                key: start,
                                frozen: false,
                                kind: node::Kind::Node3,
                            },
                            edge::Data::new_node::<Node3, _>(core::iter::once((
                                middle,
                                edge::Meta {
                                    key: end,
                                    frozen: false,
                                    kind: old_meta.kind,
                                },
                                old_data,
                            ))),
                        ),
                    )
                }
            };

            let (op, new_meta, new_data) = match unsafe { old_data.to_node(old_meta.kind) } {
                Some(Or::R(node)) => {
                    match self.history.freeze() {
                        true if node.is_frozen() => (),
                        true | false => {
                            let byte = key[old_meta.key.len.to_usize()];
                            #[allow(clippy::single_match)]
                            match node.get_or_reserve(byte) {
                                // Fast path: no need to replace
                                Ok(edge) => {
                                    self.push(old_meta.key.len, node, edge);
                                    continue;
                                }
                                Err(Frozen) => (),
                            }
                        }
                    };

                    node.freeze();
                    let (op, new_meta, new_data) = node.replace(&old_meta);
                    (Op::Node(op), new_meta, new_data)
                }
                None if key.len() > key::Len::MAX => (
                    Op::Edge(edge::Op::Create),
                    edge::Meta {
                        key: key::Array::from_slice(key),
                        frozen: false,
                        kind: node::Kind::Node3,
                    },
                    edge::Data::new_node::<Node3, _>(core::iter::empty()),
                ),
                None | Some(Or::L(_)) => (
                    Op::Edge(edge::Op::Insert),
                    edge::Meta {
                        key: key::Array::from_slice(key),
                        frozen: false,
                        kind: node::Kind::Leaf,
                    },
                    edge::Data::new_leaf(value),
                ),
            };

            return (op, (old_meta, old_data), (new_meta, new_data));
        }
    }

    fn push(&mut self, len: key::Len, node: node::Ref<'a>, edge: &'a Edge) {
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
    pub(crate) fn here(&self) -> &Edge {
        self.here
    }

    #[inline]
    pub(crate) fn key(&self) -> &'k [u8] {
        &self.prefix[self.index..]
    }
}

pub(crate) trait History<'a>: Default {
    type PopError;
    fn freeze(&mut self) -> bool;
    fn push(&mut self, segment: Segment<'a>);
    fn pop(&mut self) -> Result<Option<Segment<'a>>, Self::PopError>;
}

#[derive(Default)]
pub(crate) struct Optimistic<'a>(PhantomData<&'a ()>);

impl<'a> History<'a> for Optimistic<'a> {
    type PopError = ();

    #[inline]
    fn freeze(&mut self) -> bool {
        false
    }

    #[inline]
    fn push(&mut self, _segment: Segment<'a>) {}

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'a>>, Self::PopError> {
        Err(())
    }
}

#[derive(Default)]
pub(crate) struct Pessimistic<'a> {
    freeze: bool,
    path: Vec<Segment<'a>>,
}

impl<'a> History<'a> for Pessimistic<'a> {
    type PopError = Infallible;

    #[inline]
    fn freeze(&mut self) -> bool {
        mem::take(&mut self.freeze)
    }

    #[inline]
    fn push(&mut self, segment: Segment<'a>) {
        self.path.push(segment)
    }

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'a>>, Self::PopError> {
        self.freeze = true;
        Ok(self.path.pop())
    }
}

#[derive(Debug)]
pub(crate) struct Segment<'a> {
    len: key::Len,
    edge: &'a Edge,
    node: node::Ref<'a>,
}
