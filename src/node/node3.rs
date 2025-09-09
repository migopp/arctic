use core::fmt::Debug;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;
use ribbit::atomic::Atomic32;
use ribbit::u2;
use ribbit::u24;
use ribbit::u48;

use crate::key;
use crate::node;
use crate::node::Edge;
use crate::node::Frozen;
use crate::node::Op;
use crate::Node;

use super::Node256;

#[repr(C)]
#[derive(Debug)]
pub(crate) struct Node3 {
    header: Atomic32<Header>,

    _pad: u64,

    edges: [Atomic128<Edge>; 3],
}

const _: () = assert!(core::mem::size_of::<Node3>() == 64);

impl Node3 {
    pub(crate) fn new() -> Self {
        Self {
            header: Atomic32::new(Header::default()),
            _pad: 0,
            edges: core::array::from_fn(|_| Atomic128::new(Edge::default())),
        }
    }
}

impl Node for Node3 {
    fn get(&self, key: u8) -> Option<&Atomic128<Edge>> {
        let index = self.header.load(Ordering::Acquire).get(key);
        self.edges.get(index as usize)
    }

    fn get_or_reserve(&self, key: u8) -> Result<&Atomic128<Edge>, Frozen> {
        let mut old = self.header.load(Ordering::Acquire);

        while !old.frozen {
            let Some((new, index)) = old.get_or_reserve(key) else {
                return Err(Frozen);
            };

            match self
                .header
                .compare_exchange(old, new, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return Ok(&self.edges[index as usize]),
                Err(conflict) => old = conflict,
            }
        }

        Err(Frozen)
    }

    fn reserve(&mut self, key: u8) -> Option<&mut Atomic128<Edge>> {
        let header = self.header.get();
        let (header, index) = header.get_or_reserve(key)?;
        self.header.set(header);
        Some(&mut self.edges[index as usize])
    }

    fn is_frozen(&self) -> bool {
        self.header.load(Ordering::Relaxed).frozen
    }

    fn freeze(&self) {
        let mut old = self.header.load(Ordering::Relaxed);

        while !old.frozen {
            match self.header.compare_exchange(
                old,
                Header {
                    frozen: true,
                    ..old
                },
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(conflict) => old = conflict,
            }
        }

        self.edges
            .iter()
            .take(old.len.value() as usize)
            .for_each(Edge::freeze);
    }

    fn replace(&self, snapshot: &Edge) -> (Op, Edge) {
        let header = self.header.load(Ordering::Relaxed);
        let keys = header.keys.value();

        assert!(header.frozen);

        let mut edges: [(u8, Edge); 3] = core::array::from_fn(|_| (0, Edge::default()));
        let mut len = 0;

        self.edges
            .iter()
            .take(header.len.value() as usize)
            .map(|edge| edge.load(Ordering::Relaxed))
            .inspect(|edge| assert!(edge.frozen))
            .enumerate()
            .filter(|(_, edge)| !matches!(edge.kind, node::Kind::None))
            .map(|(index, edge)| {
                (
                    (keys >> (index * 8)) as u8,
                    Edge {
                        frozen: false,
                        ..edge
                    },
                )
            })
            .zip(&mut edges)
            .for_each(|(edge, save)| {
                *save = edge;
                len += 1;
            });

        let edges = &edges[..len];

        match edges {
            [] => (
                Op::Destroy,
                Edge {
                    key: key::Array::default(),
                    kind: node::Kind::None,
                    ..*snapshot
                },
            ),

            [(key, child)] if key::Array::can_compress(&snapshot.key, &child.key) => (
                Op::Compress,
                Edge {
                    key: key::Array::compress(&snapshot.key, *key, &child.key),
                    frozen: false,
                    ..*child
                },
            ),

            // Grow
            edges if edges.len() == 3 => {
                let mut node = Box::new(Node256::new());

                for (key, edge) in edges {
                    node.reserve(*key).unwrap().store(*edge, Ordering::Relaxed);
                }

                let node = Box::leak(node) as *mut Node256;

                (
                    node::Op::Grow,
                    Edge {
                        kind: node::Kind::Node256,
                        next: u48::new(node as u64),
                        ..*snapshot
                    },
                )
            }

            // Replace
            _ => {
                let mut node = Box::new(Node3::new());

                for (key, edge) in edges {
                    node.reserve(*key).unwrap().store(*edge, Ordering::Relaxed);
                }

                let node = Box::leak(node) as *mut Node3;

                (
                    node::Op::Replace,
                    Edge {
                        kind: node::Kind::Node3,
                        next: u48::new(node as u64),
                        ..*snapshot
                    },
                )
            }
        }
    }
}

#[derive(Copy, Clone, Debug, Default)]
#[ribbit::pack(size = 32, debug)]
struct Header {
    len: u2,
    frozen: bool,
    #[ribbit(offset = 8)]
    keys: u24,
}

impl Header {
    fn get(&self, key: u8) -> u8 {
        let keys = self.keys.value();
        let len = self.len.value();

        // https://richardstartin.github.io/posts/finding-bytes
        if cfg!(feature = "opt-node3-get") {
            const PATTERN: u32 = 0x00_7F_7F_7F;

            let input = keys ^ Self::broadcast(key);
            let temp = (input & PATTERN) + PATTERN;
            let temp = !(input | temp | PATTERN);

            (temp.trailing_zeros() >> 3) as u8
        } else {
            (0..len)
                .find(|i| (keys >> (i * 8)) as u8 == key)
                .unwrap_or(len)
        }
    }

    const fn broadcast(byte: u8) -> u32 {
        let byte = byte as u32;
        (byte << 16) | (byte << 8) | byte
    }

    fn get_or_reserve(&self, key: u8) -> Option<(Self, u8)> {
        let index = self.get(key);

        if index < self.len.value() {
            return Some((*self, index));
        }

        let keys = self.keys.value();
        let len = self.len.value();
        match len {
            0..3 => Some((
                Self {
                    len: u2::new(len + 1),
                    keys: u24::new(keys | ((key as u32) << (len * 8))),
                    frozen: self.frozen,
                },
                len,
            )),
            _ => None,
        }
    }
}

impl<'a> IntoIterator for &'a Node3 {
    type Item = (Option<u8>, Edge);
    type IntoIter = super::Iter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        let header = self.header.load(Ordering::Relaxed);
        let keys = header.keys.value();
        let keys: [u8; 3] = core::array::from_fn(|index| (keys >> (index * 8)) as u8);

        super::KeyIter::new_3(keys).zip(super::EdgeIter::new(&self.edges))
    }
}
