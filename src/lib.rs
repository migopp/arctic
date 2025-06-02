mod key;
mod node;

use core::sync::atomic::Ordering;

use node::GetOrReserveError;
pub(crate) use node::Node;
use node::Node3;
use node::Slot;
use ribbit::atomic::A128;
use ribbit::u48;
use ribbit::unpack;
use ribbit::Pack as _;

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
    index: Option<usize>,
    slot: &'a A128<Slot>,
    node: node::Ref,
}

enum Step {
    Descend { len: key::Len, node: node::Ref },
    Replace { slot: Slot },
    Stop,
}

impl Art {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, key: &[u8], value: u64) -> Option<u64> {
        eprintln!("insert {:?} = {}", key, value);
        let mut slot = &self.root;
        let mut snapshot = slot.load(Ordering::Relaxed);

        let mut index: Option<usize> = None;
        let mut path = Vec::new();

        loop {
            let key = &key[index.unwrap_or(0)..];

            eprintln!("traverse key {:?}", key);

            let (replace, leaf) = match self.get_or_insert(key, &snapshot) {
                Step::Descend { len, node } => {
                    // If `index` is None, then we are traversing from the root,
                    // and there is no byte for the node. Otherwise, we are
                    // traversing from the previous node, which takes one byte.
                    let index_node = index.map_or(0, |index| index + 1) + len.to_usize();
                    let index_slot = index_node + 1;

                    let byte = key[index_node];

                    let grow = match unsafe { node.as_node() }.get_or_reserve(byte) {
                        // Fast path: no need to replace
                        Ok(next) => {
                            path.push(Segment { index, slot, node });

                            slot = next;
                            snapshot = slot.load(Ordering::Relaxed);
                            index = Some(index_slot);
                            continue;
                        }
                        Err(GetOrReserveError::Grow) => true,
                        Err(GetOrReserveError::Freeze { grow }) => grow,
                    };

                    let node = unsafe { node.as_node() };
                    node.freeze(grow);
                    (node.replace(&snapshot), false)
                }
                Step::Replace { slot } => (slot, false),
                Step::Stop => (
                    Slot::new(
                        key::Array::from_slice(key),
                        false,
                        false,
                        node::Kind::new(<unpack![node::Kind]>::Valid),
                        u48::new(value),
                    ),
                    true,
                ),
            };

            match slot.compare_exchange(
                snapshot.with_frozen(false),
                replace,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(old) if leaf => {
                    return match old.kind().unpack() {
                        <unpack![node::Kind]>::Uninit | <unpack![node::Kind]>::Invalid => None,
                        <unpack![node::Kind]>::Valid => Some(u64::from(old.next())),
                        _ => unreachable!(),
                    }
                }
                Ok(_) => {
                    // Optimistic, can also reload from slot
                    snapshot = replace;
                }

                Err(conflict) if conflict.frozen() => {
                    todo!()
                }

                Err(conflict) => {
                    assert!(
                        conflict.key() != snapshot.key()
                            && conflict.key().len() <= snapshot.key().len()
                            || conflict.kind() != snapshot.kind()
                    );

                    snapshot = conflict;

                    // Clean up and retry
                }
            }
        }
    }

    fn get_or_insert(&self, key: &[u8], snapshot: &Slot) -> Step {
        match snapshot.traverse(key) {
            node::Traverse::Walk {
                len,
                child: node::Child::Node(child),
            } => Step::Descend { len, node: child },

            node::Traverse::Walk {
                len,
                child: node::Child::Uninit,
            } => {
                assert_eq!(len, key::Len::ZERO);

                match key.split_first_chunk::<8>() {
                    // Only create intermediate node if necessary
                    Some((head, tail)) if !tail.is_empty() => {
                        let node = Box::new(Node3::new());
                        let node = Box::leak(node) as *mut Node3;
                        let slot = Slot::new(
                            key::Array::from_slice(head),
                            false,
                            false,
                            node::Kind::new(<unpack![node::Kind]>::Node3),
                            u48::new(node as u64),
                        );

                        Step::Replace { slot }
                    }
                    Some(_) | None => Step::Stop,
                }
            }

            node::Traverse::Walk {
                len,
                child: node::Child::Leaf(_),
            } => {
                assert_eq!(key.len(), len.to_usize());

                Step::Stop
            }

            node::Traverse::Split { start, middle, end } => {
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

                Step::Replace { slot }
            }
        }
    }

    pub fn get(&self, mut key: &[u8]) -> Option<u64> {
        let mut slot = &self.root;
        eprintln!("get {:?}", key);

        loop {
            eprintln!("traverse key {:?}", key);
            match dbg!(slot.load(Ordering::Acquire).traverse(key)) {
                node::Traverse::Walk {
                    len: _,
                    child: node::Child::Uninit,
                }
                | node::Traverse::Split { .. } => break None,

                node::Traverse::Walk {
                    len,
                    child: node::Child::Leaf(leaf),
                } => {
                    assert_eq!(key.len(), len.to_usize());
                    break leaf.map(u48::value);
                }

                node::Traverse::Walk {
                    len,
                    child: node::Child::Node(node),
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
