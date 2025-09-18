use core::convert::Infallible;
use core::marker::PhantomData;
use core::mem;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;
use ribbit::Pack as _;
use ribbit::Unpack as _;

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
            let edge = self.here().load_packed(Ordering::Relaxed);
            let meta = edge.meta();
            let key = self.key();
            let len = key::Array::match_prefix(key, meta.key())?;

            match unsafe { edge.data().unpack().to_node(meta.kind().unpack()) }? {
                Or::R(node) => {
                    let byte = key.get(len)?;
                    let next = node.get(*byte)?;
                    self.push(len, node, next);
                    continue;
                }
                Or::L(leaf) => return Some(leaf),
            }
        }
    }

    pub(crate) fn traverse(&mut self) -> Option<(usize, Edge)> {
        loop {
            let edge = self.here().load_packed(Ordering::Relaxed);
            let meta = edge.meta();
            let key = self.key();

            match unsafe { edge.data().unpack().to_node(meta.kind().unpack())? } {
                // Continue traversal only if exact match
                Or::R(node) => {
                    if let Some(len) = key::Array::match_prefix(key, meta.key()) {
                        if let Some(next) = key.get(len).and_then(|byte| node.get(*byte)) {
                            self.push(len, node, next);
                            continue;
                        }
                    }
                }
                Or::L(_) => (),
            }

            return Some((self.index, edge.unpack()));
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

            let (op, new) = match key::Array::match_split(key, old_meta.key()) {
                key::Match::Full(len) => {
                    match unsafe { old.data().unpack().to_node(old_meta.kind().unpack()) } {
                        Some(Or::R(node)) => {
                            match self.history.freeze() {
                                Some(freeze) if freeze == node => (),
                                None | Some(_) => {
                                    // Must be more bytes left by no-prefix precondition
                                    let byte = key[len];

                                    #[allow(clippy::single_match)]
                                    match node.get_or_reserve(byte) {
                                        // Fast path: no need to replace
                                        Ok(edge) => {
                                            self.push(len, node, edge);
                                            continue;
                                        }
                                        Err(Frozen) => (),
                                    }
                                }
                            }

                            node.freeze();
                            let (op, new) = node.replace(old_meta);
                            (Op::Node(op), new)
                        }
                        None if key.len() > key::Array::MAX => (
                            Op::Edge(edge::Op::Create),
                            ribbit::Packed::<Edge>::new(
                                edge::Meta::NODE_3.with_key(key::Array::from_slice(key)),
                                edge::Data::new_node::<Node3, _>(None),
                            ),
                        ),
                        None | Some(Or::L(_)) => (
                            Op::Edge(edge::Op::Insert),
                            ribbit::Packed::<Edge>::new(
                                edge::Meta::LEAF.with_key(key::Array::from_slice(key)),
                                edge::Data::new_leaf(value).pack(),
                            ),
                        ),
                    }
                }
                key::Match::Partial { start, middle, end } => (
                    Op::Edge(edge::Op::Expand),
                    ribbit::Packed::<Edge>::new(
                        edge::Meta::NODE_3.with_key(start),
                        edge::Data::new_node::<Node3, _>(Some((
                            middle,
                            old.with_meta(old.meta().with_key(end).with_frozen(false)),
                        ))),
                    ),
                ),
            };

            return (op, old, new);
        }
    }

    fn push(&mut self, len: usize, node: node::Ref<'a>, edge: &'a Atomic128<Edge>) {
        self.index += len;
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
        self.index -= segment.len;
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
