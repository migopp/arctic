use core::sync::atomic::Ordering;

use crate::cursor;
use crate::cursor::Cursor;
use crate::cursor::Op;
use crate::edge;
use crate::node;
use crate::Edge;
use ribbit::atomic::Atomic128;
use ribbit::u48;
use ribbit::unpack;

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
                old.with_frozen(false),
                new,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(old) if matches!(op, Op::Edge(edge::Op::Insert)) => {
                    return Ok(old.leaf());
                }
                // FIXME: retire old allocation with SMR
                Ok(_) => continue,
                Err(conflict) => conflict,
            };

            match op {
                Op::Node(node::Op::Destroy | node::Op::Compress)
                | Op::Edge(edge::Op::Insert | edge::Op::Remove) => (),

                Op::Node(node::Op::Grow | node::Op::Replace | node::Op::Shrink)
                | Op::Edge(edge::Op::Create | edge::Op::Expand) => unsafe { new.deallocate() },
            }

            if conflict.frozen() {
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
                snapshot.with_frozen(false),
                snapshot
                    .with_frozen(false)
                    .with_kind(node::Kind::new(<unpack![node::Kind]>::None)),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(conflict) if conflict.frozen() => {
                    todo!()
                }
                Err(conflict) if conflict.key() != snapshot.key() => todo!(),
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
                snapshot.with_frozen(false),
                snapshot
                    .with_frozen(false)
                    .with_kind(node::Kind::new(<unpack![node::Kind]>::Leaf))
                    .with_next(value),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(conflict) if conflict.frozen() => todo!(),
                Err(conflict) if conflict.key() != snapshot.key() => todo!(),
                Err(conflict) => {
                    snapshot = conflict;
                }
            }
        }

        snapshot.leaf()
    }
}
