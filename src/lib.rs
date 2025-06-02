mod key;
mod node;
mod slot;

use core::sync::atomic::Ordering;

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
struct Segment<'a> {
    len: key::Len,
    slot: &'a A128<Slot>,
    node: node::Ref,
}

struct Cursor<'a> {
    index: usize,
    slot: &'a A128<Slot>,
    path: Vec<Segment<'a>>,
}

enum Direction {
    Ascend { node: node::Ref, grow: bool },
    Descend,
}

impl<'a> Cursor<'a> {
    fn new(art: &'a Art) -> Self {
        Self {
            index: 0,
            slot: &art.root,
            path: Vec::new(),
        }
    }

    fn push(&mut self, len: key::Len, node: node::Ref, slot: &'a A128<Slot>) {
        self.index += len.to_usize();
        self.index += 1;

        self.path.push(Segment {
            len,
            slot: self.slot,
            node,
        });
        self.slot = slot;
    }

    fn pop(&mut self) -> node::Ref {
        let segment = self.path.pop().unwrap();
        self.index -= 1;
        self.index -= segment.len.to_usize();
        self.slot = segment.slot;
        segment.node
    }

    fn slot(&self) -> &A128<Slot> {
        self.slot
    }

    fn key_partial<'k>(&self, key_full: &'k [u8]) -> &'k [u8] {
        &key_full[self.index..]
    }
}

enum Op {
    Node(node::Op),
    Slot(slot::Op),
}

enum Step {
    Descend { len: key::Len, node: node::Ref },
    Replace { op: slot::Op, slot: Slot },
}

impl Art {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, key: &[u8], value: u64) -> Option<u64> {
        eprintln!("insert {:?} = {}", key, value);

        // TODO: implement optimistic version of `insert`
        //
        // Insert operations that don't CAS into a frozen
        // parent slot can complete without tracking any
        // path history.
        let mut cursor = Cursor::new(self);
        let mut direction = Direction::Descend;

        loop {
            let key = cursor.key_partial(key);
            let snapshot = cursor.slot().load(Ordering::Relaxed);

            eprintln!("match key {:?}", key);

            let (op, slot) = match &direction {
                Direction::Descend => match self.get_or_replace(&snapshot, key, value) {
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

            let conflict = match cursor.slot().compare_exchange(
                snapshot.with_frozen(false),
                slot,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(old) if matches!(op, Op::Slot(slot::Op::Insert)) => {
                    return match old.kind().unpack() {
                        <unpack![node::Kind]>::Uninit | <unpack![node::Kind]>::Invalid => None,
                        <unpack![node::Kind]>::Valid => Some(u64::from(old.next())),
                        _ => unreachable!(),
                    }
                }
                Ok(_) => {
                    direction = Direction::Descend;
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
            if !conflict.frozen() {
                assert!(
                    conflict.key() != snapshot.key()
                        && conflict.key().len() <= snapshot.key().len()
                        || conflict.kind() != snapshot.kind()
                );

                // Someone else must have completed (e.g. grow)
                // or will complete (e.g. split) helping if
                // we were ascending
                direction = Direction::Descend;

                // Retry
                continue;
            }

            // Start ascending
            let node = cursor.pop();
            direction = Direction::Ascend {
                node,
                grow: conflict.grow(),
            };
        }
    }

    fn get_or_replace(&self, snapshot: &Slot, key: &[u8], value: u64) -> Step {
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

    pub fn get(&self, mut key: &[u8]) -> Option<u64> {
        let mut slot = &self.root;
        eprintln!("get {:?}", key);

        loop {
            eprintln!("match key {:?}", key);
            match dbg!(slot.load(Ordering::Acquire).r#match(key)) {
                slot::Match::Full {
                    len: _,
                    child: slot::Child::Uninit,
                }
                | slot::Match::Partial { .. } => break None,

                slot::Match::Full {
                    len,
                    child: slot::Child::Leaf(leaf),
                } => {
                    assert_eq!(key.len(), len.to_usize());
                    break leaf.map(u48::value);
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
