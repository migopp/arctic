mod node;

use core::sync::atomic::Ordering;

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

impl Art {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, mut key: &[u8], value: u64) -> Option<u64> {
        eprintln!("insert {:?} = {}", key, value);
        let mut slot = &self.root;

        loop {
            let snapshot = slot.load(Ordering::Relaxed);

            eprintln!("traverse key {:?}", key);
            match dbg!(snapshot.traverse(key)) {
                node::Traverse::Walk {
                    len,
                    child: node::Child::Uninit,
                } => {
                    assert_eq!(len, 0);

                    match key.split_first_chunk::<8>() {
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
                            prefix[..key.len()].copy_from_slice(key);
                            let prefix = u64::from_be_bytes(prefix);

                            slot.compare_exchange(
                                snapshot.with_freeze(false),
                                snapshot
                                    .with_key(prefix)
                                    .with_len(key.len() as u8)
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
                    assert_eq!(key.len(), len);

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
                    key = &key[len..];
                    let (head, tail) = key.split_first()?;
                    let node = unsafe { node.as_node() };
                    slot = node.get_or_reserve(*head).unwrap();
                    key = tail;
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
    fn node4_overwrite() {
        let art = Art::default();

        for value in [1, 2, 3, 4] {
            art.insert(&[1], value as u64);
            assert_eq!(art.get(&[1]), Some(value as u64));
        }
    }

    #[test]
    fn node4_full() {
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
    fn node4_expand() {
        let art = Art::default();

        const KEYS: [u8; 5] = [1, 2, 3, 4, 5];

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
