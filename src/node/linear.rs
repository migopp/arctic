use core::fmt::Debug;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;
use ribbit::u4;
use ribbit::u48;

use crate::key;
use crate::node;
use crate::node::Edge;
use crate::node::Frozen;
use crate::node::Op;
use crate::Node;

#[repr(C)]
#[derive(Debug)]
#[expect(private_bounds)]
pub(crate) struct Linear<const LEN: usize, K>
where
    K: KeyArray,
{
    pub(super) header: Atomic128<Header<K>>,
    pub(super) edges: [Atomic128<Edge>; LEN],
}

impl<const LEN: usize, K> Default for Linear<LEN, K>
where
    K: KeyArray,
{
    fn default() -> Self {
        Self {
            header: Atomic128::new(Header::default()),
            edges: core::array::from_fn(|_| Atomic128::new(Edge::default())),
        }
    }
}

impl<const LEN: usize, K> Node for Linear<LEN, K>
where
    K: KeyArray,
    Self: node::Info,
{
    fn get(&self, key: u8) -> Option<&Atomic128<Edge>> {
        let index = self.header.load(Ordering::Acquire).get(key);
        self.edges.get(index)
    }

    fn get_or_reserve(&self, key: u8) -> Result<&Atomic128<Edge>, Frozen> {
        let mut old = self.header.load(Ordering::Acquire);

        while !old.is_frozen() {
            let Some((new, index)) = old.get_or_reserve(key) else {
                return Err(Frozen);
            };

            match self
                .header
                .compare_exchange(old, new, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return Ok(&self.edges[index]),
                Err(conflict) => old = conflict,
            }
        }

        Err(Frozen)
    }

    fn reserve(&mut self, key: u8) -> Option<&mut Atomic128<Edge>> {
        let header = self.header.get();
        let (header, index) = header.get_or_reserve(key)?;
        self.header.set(header);
        Some(&mut self.edges[index])
    }

    fn is_frozen(&self) -> bool {
        self.header.load(Ordering::Relaxed).is_frozen()
    }

    fn freeze(&self) {
        let mut old = self.header.load(Ordering::Relaxed);

        while !old.is_frozen() {
            match self.header.compare_exchange(
                old,
                old.freeze(),
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(conflict) => old = conflict,
            }
        }

        self.edges.iter().take(old.len()).for_each(Edge::freeze);
    }

    fn replace(&self, snapshot: &Edge) -> (Op, Edge) {
        let header = self.header.load(Ordering::Relaxed);

        assert!(header.is_frozen());

        let mut edges: [(u8, Edge); LEN] = core::array::from_fn(|_| (0, Edge::default()));
        let mut len = 0;

        self.edges
            .iter()
            .map(|edge| edge.load(Ordering::Relaxed))
            .zip(header.keys())
            .inspect(|(edge, _)| assert!(edge.frozen))
            .filter(|(edge, _)| !matches!(edge.kind, node::Kind::None))
            .map(|(edge, key)| {
                (
                    key,
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
            edges if edges.len() == <Self as node::Info>::GROW => {
                let mut node = Box::new(<Self as node::Info>::Grow::default());

                for (key, edge) in edges {
                    node.reserve(*key).unwrap().store(*edge, Ordering::Relaxed);
                }

                let node = Box::leak(node) as *mut <Self as node::Info>::Grow;

                (
                    node::Op::Grow,
                    Edge {
                        kind: <<Self as node::Info>::Grow as node::Info>::KIND,
                        next: u48::new(node as u64),
                        ..*snapshot
                    },
                )
            }

            // Replace
            _ => {
                let mut node = Box::new(Self::default());

                for (key, edge) in edges {
                    node.reserve(*key).unwrap().store(*edge, Ordering::Relaxed);
                }

                let node = Box::leak(node) as *mut Self;

                (
                    node::Op::Replace,
                    Edge {
                        kind: <Self as node::Info>::KIND,
                        next: u48::new(node as u64),
                        ..*snapshot
                    },
                )
            }
        }
    }
}

#[derive(Copy, Clone, Debug, Default)]
#[ribbit::pack(size = 128)]
pub(super) struct Header<K> {
    len: u4,
    frozen: bool,
    #[ribbit(offset = 8, size = 120)]
    pub(super) keys: K,
}

impl<K: KeyArray> Header<K> {
    fn is_frozen(&self) -> bool {
        self.frozen
    }

    fn freeze(&self) -> Self {
        Self {
            frozen: true,
            ..*self
        }
    }

    fn len(&self) -> usize {
        self.len.value() as usize
    }

    fn get(&self, key: u8) -> usize {
        self.keys.get(key)
    }

    fn get_or_reserve(&self, key: u8) -> Option<(Self, usize)> {
        let index = self.get(key);
        let len = self.len();

        if index < len {
            return Some((*self, index));
        }

        if len >= K::LEN {
            return None;
        }

        Some((
            Self {
                len: u4::new((len + 1) as u8),
                keys: self.keys.insert(len, key),
                frozen: self.frozen,
            },
            len,
        ))
    }

    fn keys(&self) -> impl Iterator<Item = u8> + '_ {
        let len = self.len();
        self.keys.iter().take(len)
    }
}

pub(super) trait KeyArray: ribbit::Pack + Default {
    const LEN: usize;

    fn get(&self, key: u8) -> usize {
        self.iter()
            .position(|byte| byte == key)
            .unwrap_or(usize::MAX)
    }

    fn insert(&self, index: usize, key: u8) -> Self;
    fn iter(&self) -> impl Iterator<Item = u8>;
}
