mod node;

use core::sync::atomic::Ordering;

use node::GetOrReserveError;
pub(crate) use node::Node;
use node::Node3;
use node::Slot;
use ribbit::atomic::A128;
use ribbit::u48;
use ribbit::unpack;

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
    slot: &'a A128<Slot>,
    index: Option<usize>,
    len: usize,
}

impl Art {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, key: &[u8], value: u64) -> Option<u64> {
        eprintln!("insert {:?} = {}", key, value);
        let mut slot = &self.root;
        let mut index: Option<usize> = None;

        let mut path = Vec::new();

        loop {
            let snapshot = slot.load(Ordering::Relaxed);
            let here = &key[index.unwrap_or(0)..];

            eprintln!("traverse key {:?}", here);
            match dbg!(snapshot.traverse(here)) {
                node::Traverse::Walk {
                    len,
                    child: node::Child::Uninit,
                } => {
                    assert_eq!(len, 0);

                    match here.split_first_chunk::<8>() {
                        // Only create intermediate node if necessary
                        Some((head, tail)) if !tail.is_empty() => {
                            let node = Box::new(Node3::new());
                            let node = Box::leak(node) as *mut Node3;

                            slot.compare_exchange(
                                snapshot.with_freeze(false),
                                snapshot
                                    .with_key(u64::from_be_bytes(*head))
                                    .with_len(8)
                                    .with_freeze(false)
                                    .with_kind(node::Kind::new(<unpack![node::Kind]>::Node3))
                                    .with_next(u48::new(node as u64)),
                                Ordering::AcqRel,
                                Ordering::Acquire,
                            )
                            .unwrap();
                        }
                        Some(_) | None => {
                            let mut prefix = [0u8; 8];
                            prefix[..here.len()].copy_from_slice(here);
                            let prefix = u64::from_be_bytes(prefix);

                            slot.compare_exchange(
                                snapshot.with_freeze(false),
                                snapshot
                                    .with_key(prefix)
                                    .with_len(here.len() as u8)
                                    .with_freeze(false)
                                    .with_kind(node::Kind::new(<unpack![node::Kind]>::Valid))
                                    .with_next(u48::new(value)),
                                Ordering::AcqRel,
                                Ordering::Acquire,
                            )
                            .unwrap();

                            return None;
                        }
                    };
                }

                node::Traverse::Walk {
                    len,
                    child: node::Child::Leaf(leaf),
                } => {
                    assert_eq!(here.len(), len);

                    slot.compare_exchange(
                        snapshot.with_freeze(false),
                        snapshot.with_freeze(false).with_next(u48::new(value)),
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .unwrap();

                    return leaf.map(u48::value);
                }

                node::Traverse::Walk {
                    len,
                    child: node::Child::Node(node),
                } => {
                    path.push(Segment { slot, index, len });

                    // If `index` is None, then we are traversing from the root,
                    // and there is no byte for the node. Otherwise, we are
                    // traversing from the previous node, which takes one byte.
                    let index_node = index.map_or(0, |index| index + 1) + len;
                    let index_slot = index_node + 1;

                    let byte = key[index_node];
                    let node = unsafe { node.as_node() };

                    match node.get_or_reserve(byte) {
                        Ok(next) => {
                            slot = next;
                            index = Some(index_slot);
                        }
                        Err(GetOrReserveError::Grow) => {
                            node.grow(path.last().unwrap().slot, &snapshot).unwrap();
                        }
                        Err(GetOrReserveError::Freeze { grow: _ }) => todo!(),
                    }
                }

                node::Traverse::Split {
                    start_len,
                    end_len,
                    start,
                    middle,
                    end,
                } => {
                    let mut node = Box::new(Node3::new());

                    let old = node.reserve(middle).unwrap();
                    old.store(
                        Slot::new(
                            end,
                            end_len as u8,
                            false,
                            false,
                            snapshot.kind(),
                            snapshot.next(),
                        ),
                        Ordering::Relaxed,
                    );

                    let node = Box::leak(node) as *mut Node3;

                    slot.compare_exchange(
                        snapshot.with_freeze(false),
                        Slot::new(
                            start,
                            start_len as u8,
                            false,
                            false,
                            node::Kind::new(<unpack![node::Kind]>::Node3),
                            u48::new(node as u64),
                        ),
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .unwrap();
                }
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
                    assert_eq!(key.len(), len);
                    break leaf.map(u48::value);
                }

                node::Traverse::Walk {
                    len,
                    child: node::Child::Node(node),
                } => {
                    key = &key[len..];
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
