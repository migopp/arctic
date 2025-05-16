mod node;

use core::sync::atomic::Ordering;

pub(crate) use node::Node;
use node::Slot;
use ribbit::atomic::A128;

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
        todo!()
    }

    pub fn get(&self, mut key: &[u8]) -> Option<*mut ()> {
        // let mut path = Vec::new();
        // let mut node;
        let mut slot = &self.root;

        loop {
            match slot.load(Ordering::Relaxed).traverse(&mut key) {
                node::Traverse::Walk(node::Tree::Leaf(leaf)) if key.is_empty() => break leaf,
                node::Traverse::Walk(node::Tree::Leaf(_)) => break None,

                node::Traverse::Walk(node::Tree::Node(node)) => {
                    let (head, tail) = key.split_first()?;
                    let node = unsafe { node.as_node() };
                    slot = node.get(*head)?;
                    key = tail;
                }

                node::Traverse::Split(prefix) => todo!(),
            }
        }
    }
}

// #[cfg(test)]
// mod tests {
//     use crate::Art;
//
//     #[test]
//     fn smoke() {
//         let art = Art::default();
//         art.insert(b"abcd", 1);
//         assert_eq!(art.get(b"abcd"), Some(1));
//     }
//
//     #[test]
//     fn node4_overwrite() {
//         let art = Art::default();
//
//         for value in [1, 2, 3, 4] {
//             art.insert(&[1], value as u64);
//             assert_eq!(art.get(&[1]), Some(value as u64));
//         }
//     }
//
//     #[test]
//     fn node4_full() {
//         let art = Art::default();
//
//         const KEYS: [u8; 4] = [1, 2, 3, 4];
//
//         for key in KEYS {
//             art.insert(&[key], key as u64);
//             assert_eq!(art.get(&[key]), Some(key as u64));
//         }
//
//         for key in KEYS {
//             assert_eq!(art.get(&[key]), Some(key as u64));
//         }
//     }
//
//     #[test]
//     fn node4_expand() {
//         let art = Art::default();
//
//         const KEYS: [u8; 5] = [1, 2, 3, 4, 5];
//
//         for key in KEYS {
//             art.insert(&[key], key as u64);
//             assert_eq!(art.get(&[key]), Some(key as u64));
//         }
//
//         for key in KEYS {
//             assert_eq!(art.get(&[key]), Some(key as u64));
//         }
//     }
//
//     #[test]
//     fn node256_full() {
//         let art = Art::default();
//
//         for key in 0..=255 {
//             art.insert(&[key], key as u64);
//             assert_eq!(art.get(&[key]), Some(key as u64));
//         }
//
//         for key in 0..=255 {
//             assert_eq!(art.get(&[key]), Some(key as u64));
//         }
//     }
// }
