use core::iter;
use core::mem;
use core::sync::atomic::Ordering;
use std::rc::Rc;

use crate::cursor;
use crate::cursor::Cursor;
use crate::cursor::Op;
use crate::edge;
use crate::node;
use crate::Edge;
use ribbit::atomic::Atomic128;
use ribbit::u48;

pub struct Raw {
    root: Atomic128<Edge>,
}

impl Default for Raw {
    fn default() -> Self {
        Raw {
            root: Atomic128::new(Edge::default()),
        }
    }
}

impl Raw {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, key: &[u8], value: u48) -> Option<u48> {
        match self.insert_optimistic(key, value) {
            Ok(old) => old,
            Err(()) => self.insert_pessimistic(key, value),
        }
    }

    #[inline]
    fn insert_optimistic(&self, key: &[u8], value: u48) -> Result<Option<u48>, ()> {
        self.insert_impl::<cursor::Optimistic>(key, value)
    }

    #[cold]
    fn insert_pessimistic(&self, key: &[u8], value: u48) -> Option<u48> {
        self.insert_impl::<cursor::Pessimistic>(key, value).unwrap()
    }

    fn insert_impl<'a, P: cursor::History<'a>>(
        &'a self,
        key: &[u8],
        value: u48,
    ) -> Result<Option<u48>, P::PopError> {
        let mut cursor = Cursor::<P>::new(&self.root, key);

        loop {
            let (op, old, new) = cursor.traverse_strong(value);

            let conflict = match cursor.here().compare_exchange(
                Edge {
                    frozen: false,
                    ..old
                },
                new,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(old) => {
                    crate::stat::increment(&op);

                    if matches!(op, Op::Edge(edge::Op::Insert)) {
                        return Ok(old.leaf());
                    } else {
                        // FIXME: retire old allocation with SMR
                        continue;
                    }
                }
                Err(conflict) => conflict,
            };

            match op {
                Op::Node(node::Op::Destroy | node::Op::Compress)
                | Op::Edge(edge::Op::Insert | edge::Op::Remove) => (),

                Op::Node(node::Op::Grow | node::Op::Replace | node::Op::Shrink)
                | Op::Edge(edge::Op::Create | edge::Op::Expand) => unsafe { new.deallocate() },
            }

            if conflict.frozen {
                cursor.pop()?;
            }
        }
    }

    pub fn get(&self, key: &[u8]) -> Option<u48> {
        let mut cursor = Cursor::<cursor::Optimistic>::new(&self.root, key);
        cursor.traverse_weak()?.leaf()
    }

    pub fn remove(&self, key: &[u8]) -> Option<u48> {
        let mut cursor = Cursor::<cursor::Optimistic>::new(&self.root, key);
        let mut snapshot = cursor.traverse_weak()?;
        let edge = cursor.here();

        loop {
            match edge.compare_exchange(
                Edge {
                    frozen: false,
                    ..snapshot
                },
                Edge {
                    frozen: false,
                    kind: node::Kind::None,
                    ..snapshot
                },
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(conflict) if conflict.frozen => {
                    todo!()
                }
                Err(conflict) if conflict.key != snapshot.key => todo!(),
                Err(conflict) => {
                    snapshot = conflict;
                }
            }
        }

        snapshot.leaf()
    }

    pub fn update(&self, key: &[u8], value: u48) -> Option<u48> {
        let mut cursor = Cursor::<cursor::Optimistic>::new(&self.root, key);
        let mut snapshot = cursor.traverse_weak()?;
        let edge = cursor.here();

        loop {
            match edge.compare_exchange(
                Edge {
                    frozen: false,
                    ..snapshot
                },
                Edge {
                    frozen: false,
                    kind: node::Kind::Leaf,
                    next: value,
                    ..snapshot
                },
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(conflict) if conflict.frozen => todo!(),
                Err(conflict) if conflict.key != snapshot.key => todo!(),
                Err(conflict) => {
                    snapshot = conflict;
                }
            }
        }

        snapshot.leaf()
    }

    pub fn iter(&mut self) -> impl Iterator<Item = (Rc<Vec<u8>>, u48)> + '_ {
        self.preorder()
            .filter_map(|(_, key, edge)| match edge.child()? {
                edge::Child::Leaf => Some((key, edge.next)),
                edge::Child::Node(_) => None,
            })
    }

    pub fn keys(&mut self) -> impl Iterator<Item = Rc<Vec<u8>>> + '_ {
        self.iter().map(|(key, _)| key)
    }

    pub fn values(&mut self) -> impl Iterator<Item = u48> + '_ {
        self.iter().map(|(_, value)| value)
    }

    pub(crate) fn preorder(&mut self) -> impl Iterator<Item = (usize, Rc<Vec<u8>>, Edge)> + '_ {
        Iter::new(&mut self.root)
    }
}

struct Iter<'a> {
    // Workaround for lending iterator
    // https://users.rust-lang.org/t/how-to-write-an-iterator-that-returns-references-to-itself/72386/5
    key: Rc<Vec<u8>>,

    // TODO: allow starting traversal at a given prefix?
    frontier: Vec<(
        usize,
        iter::Peekable<iter::Zip<iter::Repeat<bool>, node::Iter<'a>>>,
    )>,
}

impl<'a> Iter<'a> {
    fn new(root: &'a mut Atomic128<Edge>) -> Self {
        Self {
            key: Rc::new(Vec::new()),
            frontier: vec![(
                0,
                iter::repeat(false)
                    .zip(
                        node::KeyIter::new_0()
                            .zip(node::EdgeIter::new(core::slice::from_ref(root))),
                    )
                    .peekable(),
            )],
        }
    }
}

impl<'a> Iterator for Iter<'a> {
    type Item = (usize, Rc<Vec<u8>>, Edge);

    fn next(&mut self) -> Option<Self::Item> {
        'vertical: loop {
            let (depth, _) = self.frontier.len().overflowing_sub(1);
            let (len, node) = self.frontier.last_mut()?;

            'horizontal: loop {
                let Some((descend, (byte, edge))) = node.peek_mut() else {
                    Rc::make_mut(&mut self.key).truncate(*len);
                    self.frontier.pop();
                    continue 'vertical;
                };

                // Skip empty edges
                let Some(child) = edge.child() else {
                    node.next();
                    continue 'horizontal;
                };

                // Update key for current edge
                let byte = *byte;
                let edge = *edge;
                let key = Rc::make_mut(&mut self.key);

                // Produce edge before traversing for preorder traversal
                if !mem::replace(descend, true) {
                    key.extend(byte.into_iter().chain(edge.key.bytes()));
                    return Some((depth, Rc::clone(&self.key), edge));
                }

                node.next();
                let len = key.len() - edge.key.len.to_usize() - byte.is_some() as usize;

                match child {
                    edge::Child::Leaf => {
                        key.truncate(len);
                        continue 'horizontal;
                    }
                    edge::Child::Node(child) => {
                        self.frontier.push((
                            len,
                            iter::repeat(false).zip(unsafe { child.iter() }).peekable(),
                        ));
                        continue 'vertical;
                    }
                }
            }
        }
    }
}
