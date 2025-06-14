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
    path: P,
}

pub(crate) enum Op {
    Node(node::Op),
    Slot(slot::Op),
}

impl<'a, 'k, P: Path<'a>> Cursor<'a, 'k, P> {
    pub(crate) fn new(root: &'a A128<Slot>, key: &'k [u8]) -> Self {
        Self {
            key,
            index: 0,
            here: root,
            path: P::default(),
        }
    }

    pub(crate) fn get(&mut self) -> Option<Slot> {
        loop {
            let slot = self.here();
            let snapshot = slot.load(Ordering::Acquire);
            let key = self.key();

            match snapshot.r#match(key) {
                slot::Match::Full {
                    len: _,
                    child: slot::Child::Uninit,
                }
                | slot::Match::Partial { .. } => return None,

                slot::Match::Full {
                    len,
                    child: slot::Child::Leaf(_),
                } => {
                    assert_eq!(key.len(), len.to_usize());
                    return Some(snapshot);
                }

                slot::Match::Full {
                    len,
                    child: slot::Child::Node(node),
                } => {
                    let byte = key.get(len.to_usize())?;
                    let next = unsafe { node.as_node() }.get(*byte)?;
                    self.push(len, node, next);
                }
            }
        }
    }

    pub(crate) fn insert(&mut self, value: u48) -> (Op, Slot, Slot) {
        loop {
            let slot = self.here();
            let snapshot = slot.load(Ordering::Acquire);
            let key = self.key();

            match snapshot.r#match(key) {
                slot::Match::Full {
                    len,
                    child: slot::Child::Node(node),
                } => {
                    let byte = key[len.to_usize()];

                    let grow = match unsafe { node.as_node() }.get_or_reserve(byte) {
                        // Fast path: no need to replace
                        Ok(slot) => {
                            self.push(len, node, slot);
                            continue;
                        }
                        Err(Frozen::Grow) => true,
                        Err(Frozen::Shrink) => false,
                    };

                    let node = unsafe { node.as_node() };
                    node.freeze(grow);
                    let (op, slot) = node.replace(&snapshot);
                    return (Op::Node(op), snapshot, slot);
                }

                slot::Match::Full {
                    len: _,
                    child: slot::Child::Leaf(_) | slot::Child::Uninit,
                } if key.len() <= key::Len::MAX.to_usize() => {
                    return (
                        Op::Slot(slot::Op::Insert),
                        snapshot,
                        Slot::new(
                            key::Array::from_slice(key),
                            false,
                            false,
                            node::Kind::new(<unpack![node::Kind]>::Valid),
                            value,
                        ),
                    )
                }

                slot::Match::Full {
                    len: _,
                    child: slot::Child::Leaf(_),
                } => unreachable!(),

                slot::Match::Full {
                    len,
                    child: slot::Child::Uninit,
                } => {
                    assert_eq!(len, key::Len::ZERO);

                    let node = Box::new(Node3::new());
                    let node = Box::leak(node) as *mut Node3;
                    let slot = Slot::new(
                        key::Array::from_slice(&key[..key::Len::MAX.to_usize()]),
                        false,
                        false,
                        node::Kind::new(<unpack![node::Kind]>::Node3),
                        u48::new(node as u64),
                    );

                    return (Op::Slot(slot::Op::Create), snapshot, slot);
                }

                slot::Match::Partial { start, middle, end } => {
                    let mut node = Box::new(Node3::new());

                    let old = node.reserve(middle).unwrap();
                    old.store(
                        Slot::new(end, false, false, snapshot.kind(), snapshot.next()),
                        Ordering::Relaxed,
                    );

                    let node = Box::leak(node) as *mut Node3;
                    let slot = Slot::new(
                        start,
                        false,
                        false,
                        node::Kind::new(<unpack![node::Kind]>::Node3),
                        u48::new(node as u64),
                    );

                    return (Op::Slot(slot::Op::Expand), snapshot, slot);
                }
            }
        }
    }

    pub(crate) fn push(&mut self, len: key::Len, node: node::Ref, slot: &'a A128<Slot>) {
        self.index += len.to_usize();
        self.index += 1;
        self.path.push(Segment {
            len,
            slot: self.here,
            node,
        });
        self.here = slot;
    }

    pub(crate) fn pop(&mut self) -> Result<node::Ref, P::PopError> {
        let segment = self.path.pop()?.expect("Root slot can never be frozen");
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
    pub(crate) fn key(&self) -> &[u8] {
        &self.key[self.index..]
    }
}

pub(crate) trait Path<'a>: Default {
    type PopError;
    fn push(&mut self, segment: Segment<'a>);
    fn pop(&mut self) -> Result<Option<Segment<'a>>, Self::PopError>;
}

#[derive(Default)]
pub(crate) struct Optimistic<'a>(PhantomData<&'a ()>);

impl<'a> Path<'a> for Optimistic<'a> {
    type PopError = ();

    #[inline]
    fn push(&mut self, _segment: Segment<'a>) {}

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'a>>, Self::PopError> {
        Err(())
    }
}

#[derive(Default)]
pub(crate) struct Pessimistic<'a>(Vec<Segment<'a>>);

impl<'a> Path<'a> for Pessimistic<'a> {
    type PopError = Infallible;

    #[inline]
    fn push(&mut self, segment: Segment<'a>) {
        self.0.push(segment)
    }

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'a>>, Self::PopError> {
        Ok(self.0.pop())
    }
}

#[derive(Debug)]
struct Segment<'a> {
    len: key::Len,
    slot: &'a A128<Slot>,
    node: node::Ref,
}
