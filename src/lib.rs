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
        let mut slot = &self.root;

        loop {
            let snapshot = slot.load(Ordering::Relaxed);

            match snapshot.traverse(key) {
                node::Traverse::Walk {
                    len,
                    child: node::Child::Uninit,
                } => {
                    assert_eq!(len, 0);

                    let prefix = match key.split_first_chunk::<8>() {
                        None => {
                            let mut prefix = [0u8; 8];
                            prefix[..key.len()].copy_from_slice(key);
                            let prefix = u64::from_be_bytes(prefix);

                            slot.compare_exchange(
                                snapshot,
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
                        Some((prefix, _)) => u64::from_be_bytes(*prefix),
                    };

                    let node = Box::new(Node3::new());
                    let node = Box::leak(node) as *mut Node3;

                    slot.compare_exchange(
                        snapshot,
                        snapshot
                            .with_key(prefix)
                            .with_len(8)
                            .with_freeze(false)
                            .with_kind(node::Kind::new(<unpack![node::Kind]>::Node3))
                            .with_next(u48::new(node as u64)),
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .unwrap();
                }

                node::Traverse::Walk {
                    len,
                    child: node::Child::Leaf(leaf),
                } => {
                    assert_eq!(key.len(), len);

                    slot.compare_exchange(
                        snapshot,
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
                    slot = node.get(*head)?;
                    key = tail;
                }

                node::Traverse::Split { len } => todo!(),
            }
        }
    }

    pub fn get(&self, mut key: &[u8]) -> Option<u64> {
        // let mut path = Vec::new();
        // let mut node;
        let mut slot = &self.root;

        loop {
            match slot.load(Ordering::Acquire).traverse(key) {
                node::Traverse::Walk {
                    len: _,
                    child: node::Child::Uninit,
                }
                | node::Traverse::Split { len: _ } => break None,

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
