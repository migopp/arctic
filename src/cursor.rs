use core::convert::Infallible;
use core::marker::PhantomData;
use core::mem;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;
use ribbit::u48;

use crate::edge;
use crate::key;
use crate::node;
use crate::node::Frozen;
use crate::node::Node3;
use crate::Edge;
use crate::Node as _;

pub(crate) struct Cursor<'a, 'k, P> {
    key: &'k [u8],
    index: usize,
    here: &'a Atomic128<Edge>,
    history: P,
}

#[derive(Debug)]
pub(crate) enum Op {
    Node(node::Op),
    Edge(edge::Op),
}

impl<'a, 'k, P: History<'a>> Cursor<'a, 'k, P> {
    pub(crate) fn new(root: &'a Atomic128<Edge>, key: &'k [u8]) -> Self {
        Self {
            key,
            index: 0,
            here: root,
            history: P::default(),
        }
    }

    pub(crate) fn traverse_weak(&mut self) -> Option<Edge> {
        loop {
            let edge = self.here();
            let snapshot = edge.load(Ordering::Acquire);
            let key = self.key();

            match snapshot.r#match(key) {
                edge::Match::Full {
                    len: _,
                    child: None,
                }
                | edge::Match::Partial { .. } => return None,

                edge::Match::Full {
                    len,
                    child: Some(edge::Child::Leaf),
                } => {
                    assert_eq!(key.len(), len.to_usize());
                    return Some(snapshot);
                }

                edge::Match::Full {
                    len,
                    child: Some(edge::Child::Node(node)),
                } => {
                    let byte = key.get(len.to_usize())?;
                    let next = unsafe { node.as_node() }.get(*byte)?;
                    self.push(len, node, next);
                }
            }
        }
    }

    pub(crate) fn traverse_strong(&mut self, value: u48) -> (Op, Edge, Edge) {
        loop {
            let edge = self.here();
            let old = edge.load(Ordering::Acquire);
            let key = self.key();

            let (op, new) = match old.r#match(key) {
                edge::Match::Full {
                    len,
                    child: Some(edge::Child::Node(node)),
                } => {
                    match self.history.freeze() {
                        true if unsafe { node.as_node() }.is_frozen() => (),
                        true | false => {
                            let byte = key[len.to_usize()];
                            match unsafe { node.as_node() }.get_or_reserve(byte) {
                                // Fast path: no need to replace
                                Ok(edge) => {
                                    self.push(len, node, edge);
                                    continue;
                                }
                                Err(Frozen) => (),
                            }
                        }
                    };

                    let node = unsafe { node.as_node() };
                    node.freeze();
                    let (op, new) = node.replace(&old);
                    (Op::Node(op), new)
                }

                edge::Match::Full { len, child: None } if key.len() > key::Len::MAX.to_usize() => {
                    assert_eq!(len, key::Len::ZERO);

                    let node = Box::new(Node3::default());
                    let node = Box::leak(node) as *mut Node3;
                    let new = Edge {
                        key: key::Array::from_slice(&key[..key::Len::MAX.to_usize()]),
                        frozen: false,
                        kind: node::Kind::Node3,
                        next: u48::new(node as u64),
                    };

                    (Op::Edge(edge::Op::Create), new)
                }

                edge::Match::Full {
                    len: _,
                    child: Some(edge::Child::Leaf) | None,
                } => (
                    Op::Edge(edge::Op::Insert),
                    Edge {
                        key: key::Array::from_slice(key),
                        frozen: false,
                        kind: node::Kind::Leaf,
                        next: value,
                    },
                ),

                edge::Match::Partial { start, middle, end } => {
                    let mut node = Box::new(Node3::default());

                    node.reserve(middle).unwrap().store(
                        Edge {
                            key: end,
                            frozen: false,
                            ..old
                        },
                        Ordering::Relaxed,
                    );

                    let node = Box::leak(node) as *mut Node3;

                    let new = Edge {
                        key: start,
                        frozen: false,
                        kind: node::Kind::Node3,
                        next: u48::new(node as u64),
                    };

                    (Op::Edge(edge::Op::Expand), new)
                }
            };

            return (op, old, new);
        }
    }

    fn push(&mut self, len: key::Len, node: node::Ref, edge: &'a Atomic128<Edge>) {
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
        &self.key[self.index..]
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
    edge: &'a Atomic128<Edge>,
    node: node::Ref,
}
