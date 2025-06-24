use core::convert::Infallible;
use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use ribbit::atomic::A128;
use ribbit::u48;
use ribbit::unpack;

use crate::key;
use crate::node;
use crate::node::Frozen;
use crate::node::Node3;
use crate::slot;
use crate::Node as _;
use crate::Slot;

pub(crate) struct Cursor<'a, 'k, P> {
    key: &'k [u8],
    index: usize,
    here: &'a A128<Slot>,
    history: P,
}

pub(crate) enum Op {
    Node(node::Op),
    Slot(slot::Op),
}

impl<'a, 'k, P: History<'a>> Cursor<'a, 'k, P> {
    pub(crate) fn new(root: &'a A128<Slot>, key: &'k [u8]) -> Self {
        Self {
            key,
            index: 0,
            here: root,
            history: P::default(),
        }
    }

    pub(crate) fn traverse_weak(&mut self) -> Option<Slot> {
        loop {
            let slot = self.here();
            let snapshot = slot.load(Ordering::Acquire);
            let key = self.key();

            match snapshot.r#match(key) {
                slot::Match::Full {
                    len: _,
                    child: None,
                }
                | slot::Match::Partial { .. } => return None,

                slot::Match::Full {
                    len,
                    child: Some(slot::Child::Leaf),
                } => {
                    assert_eq!(key.len(), len.to_usize());
                    return Some(snapshot);
                }

                slot::Match::Full {
                    len,
                    child: Some(slot::Child::Node(node)),
                } => {
                    let byte = key.get(len.to_usize())?;
                    let next = unsafe { node.as_node() }.get(*byte)?;
                    self.push(len, node, next);
                }
            }
        }
    }

    pub(crate) fn traverse_strong(&mut self, value: u48) -> (Op, Slot, Slot) {
        loop {
            let slot = self.here();
            let old = slot.load(Ordering::Acquire);
            let key = self.key();

            let (op, new) = match old.r#match(key) {
                slot::Match::Full {
                    len,
                    child: Some(slot::Child::Node(node)),
                } => {
                    let grow = match self.history.freeze() {
                        Some(grow) if unsafe { node.as_node() }.is_frozen() == Some(grow) => grow,
                        Some(_) | None => {
                            let byte = key[len.to_usize()];
                            match unsafe { node.as_node() }.get_or_reserve(byte) {
                                // Fast path: no need to replace
                                Ok(slot) => {
                                    self.push(len, node, slot);
                                    continue;
                                }
                                Err(Frozen::Grow) => true,
                                Err(Frozen::Shrink) => false,
                            }
                        }
                    };

                    let node = unsafe { node.as_node() };
                    node.freeze(grow);
                    let (op, new) = node.replace(&old);
                    (Op::Node(op), new)
                }

                slot::Match::Full { len, child: None } if key.len() > key::Len::MAX.to_usize() => {
                    assert_eq!(len, key::Len::ZERO);

                    let node = Box::new(Node3::new());
                    let node = Box::leak(node) as *mut Node3;
                    let new = Slot::new(
                        key::Array::from_slice(&key[..key::Len::MAX.to_usize()]),
                        false,
                        false,
                        node::Kind::new(<unpack![node::Kind]>::Node3),
                        u48::new(node as u64),
                    );

                    (Op::Slot(slot::Op::Create), new)
                }

                slot::Match::Full {
                    len: _,
                    child: Some(slot::Child::Leaf) | None,
                } => (
                    Op::Slot(slot::Op::Insert),
                    Slot::new(
                        key::Array::from_slice(key),
                        false,
                        false,
                        node::Kind::new(<unpack![node::Kind]>::Leaf),
                        value,
                    ),
                ),

                slot::Match::Partial { start, middle, end } => {
                    let mut node = Box::new(Node3::new());

                    node.reserve(middle).unwrap().store(
                        Slot::new(end, false, false, old.kind(), old.next()),
                        Ordering::Relaxed,
                    );

                    let node = Box::leak(node) as *mut Node3;

                    let new = Slot::new(
                        start,
                        false,
                        false,
                        node::Kind::new(<unpack![node::Kind]>::Node3),
                        u48::new(node as u64),
                    );

                    (Op::Slot(slot::Op::Expand), new)
                }
            };

            return (op, old, new);
        }
    }

    fn push(&mut self, len: key::Len, node: node::Ref, slot: &'a A128<Slot>) {
        self.index += len.to_usize();
        self.index += 1;
        self.history.push(Segment {
            len,
            slot: self.here,
            node,
        });
        self.here = slot;
    }

    pub(crate) fn pop(&mut self, grow: bool) -> Result<node::Ref, P::PopError> {
        let segment = self
            .history
            .pop(grow)?
            .expect("Root slot can never be frozen");
        self.index -= 1;
        self.index -= segment.len.to_usize();
        self.here = segment.slot;
        Ok(segment.node)
    }

    #[inline]
    pub(crate) fn here(&self) -> &A128<Slot> {
        self.here
    }

    #[inline]
    pub(crate) fn key(&self) -> &'k [u8] {
        &self.key[self.index..]
    }
}

pub(crate) trait History<'a>: Default {
    type PopError;
    fn freeze(&mut self) -> Option<bool>;
    fn push(&mut self, segment: Segment<'a>);
    fn pop(&mut self, grow: bool) -> Result<Option<Segment<'a>>, Self::PopError>;
}

#[derive(Default)]
pub(crate) struct Optimistic<'a>(PhantomData<&'a ()>);

impl<'a> History<'a> for Optimistic<'a> {
    type PopError = ();

    fn freeze(&mut self) -> Option<bool> {
        None
    }

    #[inline]
    fn push(&mut self, _segment: Segment<'a>) {}

    #[inline]
    fn pop(&mut self, _grow: bool) -> Result<Option<Segment<'a>>, Self::PopError> {
        Err(())
    }
}

#[derive(Default)]
pub(crate) struct Pessimistic<'a> {
    grow: Option<bool>,
    path: Vec<Segment<'a>>,
}

impl<'a> History<'a> for Pessimistic<'a> {
    type PopError = Infallible;

    fn freeze(&mut self) -> Option<bool> {
        self.grow.take()
    }

    #[inline]
    fn push(&mut self, segment: Segment<'a>) {
        self.path.push(segment)
    }

    #[inline]
    fn pop(&mut self, grow: bool) -> Result<Option<Segment<'a>>, Self::PopError> {
        self.grow = Some(grow);
        Ok(self.path.pop())
    }
}

#[derive(Debug)]
pub(crate) struct Segment<'a> {
    len: key::Len,
    slot: &'a A128<Slot>,
    node: node::Ref,
}
