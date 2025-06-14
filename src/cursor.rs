use core::sync::atomic::Ordering;

use ribbit::atomic::A128;

use crate::key;
use crate::node;
use crate::slot;
use crate::Slot;

pub(crate) struct Cursor<'a, const OPTIMISTIC: bool> {
    key: &'a [u8],
    index: usize,
    here: &'a A128<Slot>,
    path: Vec<Segment<'a>>,
    direction: Direction,
}

pub(crate) enum Direction {
    Ascend { node: node::Ref, grow: bool },
    Descend,
}

impl<'a, const OPTIMISTIC: bool> Cursor<'a, OPTIMISTIC> {
    pub(crate) fn new(root: &'a A128<Slot>, key: &'a [u8]) -> Self {
        Self {
            key,
            index: 0,
            here: root,
            path: Vec::new(),
            direction: Direction::Descend,
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

    pub(crate) fn push(&mut self, len: key::Len, node: node::Ref, slot: &'a A128<Slot>) {
        self.index += len.to_usize();
        self.index += 1;

        if !OPTIMISTIC {
            self.path.push(Segment {
                len,
                slot: self.here,
                node,
            });
        }

        self.here = slot;
    }

    pub(crate) fn pop(&mut self, grow: bool) -> bool {
        if OPTIMISTIC {
            return false;
        }

        let segment = self.path.pop().unwrap();
        self.index -= 1;
        self.index -= segment.len.to_usize();
        self.here = segment.slot;
        self.direction = Direction::Ascend {
            node: segment.node,
            grow,
        };
        true
    }

    #[inline]
    pub(crate) fn here(&self) -> &A128<Slot> {
        self.here
    }

    #[inline]
    pub(crate) fn key(&self) -> &[u8] {
        &self.key[self.index..]
    }

    #[inline]
    pub(crate) fn direction(&self) -> &Direction {
        match OPTIMISTIC {
            true => &Direction::Descend,
            false => &self.direction,
        }
    }

    #[inline]
    pub(crate) fn descend(&mut self) {
        if OPTIMISTIC {
            return;
        }

        self.direction = Direction::Descend;
    }
}

#[derive(Debug)]
struct Segment<'a> {
    len: key::Len,
    slot: &'a A128<Slot>,
    node: node::Ref,
}
