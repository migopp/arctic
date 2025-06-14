mod cursor;
mod key;
mod node;
mod slot;

use core::sync::atomic::Ordering;

use cursor::Cursor;
use cursor::Direction;
use node::GetOrReserveError;
pub(crate) use node::Node;
use node::Node3;
use ribbit::atomic::A128;
use ribbit::u48;
use ribbit::unpack;
pub(crate) use slot::Slot;

pub struct Art {
    root: A128<Slot>,
}

impl Default for Art {
    fn default() -> Self {
        Art {
            root: A128::new(Slot::default()),
        }
    }
}

#[derive(Debug)]
enum Op {
    Node(node::Op),
    Slot(slot::Op),
}

#[derive(Debug)]
enum Step {
    Descend { len: key::Len, node: node::Ref },
    Replace { op: slot::Op, slot: Slot },
}

impl Art {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, key: &[u8], value: u64) -> Option<u64> {
        match self.insert_optimistic(key, value) {
            Ok(old) => old,
            Err(()) => self.insert_pessimistic(key, value),
        }
    }

    #[inline]
    fn insert_optimistic(&self, key: &[u8], value: u64) -> Result<Option<u64>, ()> {
        self.insert_impl::<true>(key, value)
    }

    #[cold]
    fn insert_pessimistic(&self, key: &[u8], value: u64) -> Option<u64> {
        self.insert_impl::<false>(key, value).unwrap()
    }

    fn insert_impl<const OPTIMISTIC: bool>(
        &self,
        key: &[u8],
        value: u64,
    ) -> Result<Option<u64>, ()> {
        let mut cursor = Cursor::<OPTIMISTIC>::new(&self.root, key);

        loop {
            let key = cursor.key_partial(key);
            let snapshot = cursor.here().load(Ordering::Acquire);

            let (op, slot) = match cursor.direction() {
                Direction::Descend => match self.step(&snapshot, key, value) {
                    Step::Replace { op, slot } => (Op::Slot(op), slot),
                    Step::Descend { len, node } => {
                        let byte = key[len.to_usize()];

                        let grow = match unsafe { node.as_node() }.get_or_reserve(byte) {
                            // Fast path: no need to replace
                            Ok(slot) => {
                                cursor.push(len, node, slot);
                                continue;
                            }
                            Err(GetOrReserveError::Grow) => true,
                            Err(GetOrReserveError::Freeze { grow }) => grow,
                        };

                        let node = unsafe { node.as_node() };
                        node.freeze(grow);
                        let (op, slot) = node.replace(&snapshot);
                        (Op::Node(op), slot)
                    }
                },
                Direction::Ascend { node, grow } => {
                    let node = unsafe { node.as_node() };
                    node.freeze(*grow);
                    let (op, slot) = node.replace(&snapshot);
                    (Op::Node(op), slot)
                }
            };

            let conflict = match cursor.here().compare_exchange(
                snapshot.with_frozen(false),
                slot,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(old) if matches!(op, Op::Slot(slot::Op::Insert)) => {
                    return match old.kind().unpack() {
                        <unpack![node::Kind]>::Uninit | <unpack![node::Kind]>::Invalid => Ok(None),
                        <unpack![node::Kind]>::Valid => Ok(Some(u64::from(old.next()))),
                        _ => unreachable!(),
                    };
                }
                // FIXME: retire old allocation with SMR
                Ok(_) => {
                    cursor.descend();
                    continue;
                }
                Err(conflict) => conflict,
            };

            match op {
                Op::Node(node::Op::Destroy | node::Op::Compress)
                | Op::Slot(slot::Op::Insert | slot::Op::Remove) => (),

                Op::Node(node::Op::Grow | node::Op::Replace | node::Op::Shrink)
                | Op::Slot(slot::Op::Create | slot::Op::Expand) => unsafe { slot.deallocate() },
            }

            // Conflicts can be due to:
            // - Freeze
            // - Split
            // - Initialize
            // - Leaf update
            match conflict.frozen() {
                false => {
                    // Someone else must have completed (e.g. grow)
                    // or will complete (e.g. split) helping if
                    // we were ascending
                    cursor.descend();
                }
                true if cursor.pop(conflict.grow()) => (),
                true => return Err(()),
            }
        }
    }

    pub fn get(&self, key: &[u8]) -> Option<u64> {
        let (_, snapshot) = self.walk(key)?;
        match snapshot.kind().unpack() {
            <unpack![node::Kind]>::Uninit | <unpack![node::Kind]>::Invalid => None,
            <unpack![node::Kind]>::Valid => Some(u64::from(snapshot.next())),
            _ => unreachable!(),
        }
    }

    pub fn remove(&self, key: &[u8]) -> Option<u64> {
        let (slot, mut snapshot) = self.walk(key)?;

        loop {
            match slot.compare_exchange(
                snapshot.with_frozen(false),
                snapshot
                    .with_frozen(false)
                    .with_kind(node::Kind::new(<unpack![node::Kind]>::Invalid)),
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

        match snapshot.kind().unpack() {
            <unpack![node::Kind]>::Uninit | <unpack![node::Kind]>::Invalid => None,
            <unpack![node::Kind]>::Valid => Some(u64::from(snapshot.next())),
            _ => unreachable!(),
        }
    }

    pub fn update(&self, key: &[u8], value: u64) -> Option<u64> {
        let (slot, mut snapshot) = self.walk(key)?;

        loop {
            match slot.compare_exchange(
                snapshot.with_frozen(false),
                snapshot
                    .with_frozen(false)
                    .with_kind(node::Kind::new(<unpack![node::Kind]>::Valid))
                    .with_next(u48::new(value)),
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

        match snapshot.kind().unpack() {
            <unpack![node::Kind]>::Uninit | <unpack![node::Kind]>::Invalid => None,
            <unpack![node::Kind]>::Valid => Some(u64::from(snapshot.next())),
            _ => unreachable!(),
        }
    }

    fn step(&self, snapshot: &Slot, key: &[u8], value: u64) -> Step {
        match snapshot.r#match(key) {
            slot::Match::Full {
                len,
                child: slot::Child::Node(child),
            } => Step::Descend { len, node: child },

            slot::Match::Full {
                len: _,
                child: slot::Child::Leaf(_) | slot::Child::Uninit,
            } if key.len() <= key::Len::MAX.to_usize() => Step::Replace {
                op: slot::Op::Insert,
                slot: Slot::new(
                    key::Array::from_slice(key),
                    false,
                    false,
                    node::Kind::new(<unpack![node::Kind]>::Valid),
                    u48::new(value),
                ),
            },

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

                Step::Replace {
                    op: slot::Op::Create,
                    slot,
                }
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

                Step::Replace {
                    op: slot::Op::Expand,
                    slot,
                }
            }
        }
    }

    fn walk(&self, mut key: &[u8]) -> Option<(&A128<Slot>, Slot)> {
        let mut slot = &self.root;

        loop {
            let snapshot = slot.load(Ordering::Acquire);
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
                    return Some((slot, snapshot));
                }

                slot::Match::Full {
                    len,
                    child: slot::Child::Node(node),
                } => {
                    key = &key[len.to_usize()..];
                    let (head, tail) = key.split_first()?;
                    let node = unsafe { node.as_node() };
                    slot = node.get(*head)?;
                    key = tail;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Art;

    #[test]
    fn smoke() {
        let art = Art::default();
        art.insert(b"abcd", 1);
        assert_eq!(art.get(b"abcd"), Some(1));
    }

    #[test]
    fn smoke_u64_key() {
        let art = Art::default();
        let key = 0xdeadbeefu64.to_be_bytes();
        art.insert(&key, 1);
        assert_eq!(art.get(&key), Some(1));
    }

    #[test]
    fn node3_overwrite() {
        let art = Art::default();

        for value in [1, 2, 3] {
            art.insert(&[1], value as u64);
            assert_eq!(art.get(&[1]), Some(value as u64));
        }
    }

    #[test]
    fn node3_full() {
        let art = Art::default();

        const KEYS: [u8; 3] = [1, 2, 3];

        for key in KEYS {
            art.insert(&[key], key as u64);
            assert_eq!(art.get(&[key]), Some(key as u64));
        }

        for key in KEYS {
            assert_eq!(art.get(&[key]), Some(key as u64));
        }
    }

    #[test]
    fn node3_expand() {
        let art = Art::default();

        const KEYS: [u8; 4] = [1, 2, 3, 4];

        for key in KEYS {
            art.insert(&[key], key as u64);
            assert_eq!(art.get(&[key]), Some(key as u64));
        }

        for key in KEYS {
            assert_eq!(art.get(&[key]), Some(key as u64));
        }
    }

    #[test]
    fn node256_full() {
        let art = Art::default();

        for key in 0..=255 {
            art.insert(&[key], key as u64);
            assert_eq!(art.get(&[key]), Some(key as u64));
        }

        for key in 0..=255 {
            assert_eq!(art.get(&[key]), Some(key as u64));
        }
    }
}
