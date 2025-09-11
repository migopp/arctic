use core::fmt::Debug;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;
use ribbit::u4;

use crate::edge;
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
    pub(super) edges: [Edge; LEN],
}

impl<const LEN: usize, K> Default for Linear<LEN, K>
where
    K: KeyArray,
{
    fn default() -> Self {
        Self {
            header: Atomic128::new(Header::default()),
            edges: core::array::from_fn(|_| Edge::default()),
        }
    }
}

impl<const LEN: usize, K> Node for Linear<LEN, K>
where
    K: KeyArray,
    Self: node::Info,
{
    fn get(&self, key: u8) -> Option<&Edge> {
        let index = self.header.load(Ordering::Acquire).get(key);
        self.edges.get(index)
    }

    fn get_or_reserve(&self, key: u8) -> Result<&Edge, Frozen> {
        let mut old = self.header.load(Ordering::Acquire);

        while !old.is_frozen() {
            let (new, index) = match old.get_or_reserve(key) {
                Reservation::Found(index) => return Ok(&self.edges[index]),
                Reservation::Grow => return Err(Frozen),
                Reservation::Reserve(new, index) => (new, index),
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

    fn reserve(&mut self, key: u8) -> Option<&mut Edge> {
        let header = self.header.get();
        match header.get_or_reserve(key) {
            Reservation::Found(index) => Some(&mut self.edges[index]),
            Reservation::Grow => None,
            Reservation::Reserve(header, index) => {
                self.header.set(header);
                Some(&mut self.edges[index])
            }
        }
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
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(conflict) => old = conflict,
            }
        }

        self.edges.iter().take(old.len()).for_each(Edge::freeze);
    }

    fn replace(&self, snapshot: &edge::Meta) -> (Op, edge::Meta, edge::Data) {
        let header = self.header.load(Ordering::Relaxed);

        assert!(header.is_frozen());

        let mut edges: [(u8, edge::Meta, edge::Data); LEN] =
            core::array::from_fn(|_| (0, edge::Meta::default(), edge::Data::default()));
        let mut len = 0;

        self.edges
            .iter()
            .map(|edge| (edge, edge.load_low(Ordering::Relaxed)))
            .zip(header.keys())
            .inspect(|((_, meta), _)| assert!(meta.frozen))
            .filter(|((_, meta), _)| !matches!(meta.kind, node::Kind::None))
            .map(|((edge, meta), key)| (key, meta.unfreeze(), edge.load_high(Ordering::Relaxed)))
            .zip(&mut edges)
            .for_each(|(edge, save)| {
                *save = edge;
                len += 1;
            });

        match &edges[..len] {
            [] => (
                Op::Destroy,
                edge::Meta {
                    key: key::Array::default(),
                    kind: node::Kind::None,
                    frozen: false,
                },
                edge::Data::default(),
            ),

            [(key, meta, data)] if key::Array::can_compress(&snapshot.key, &meta.key) => (
                Op::Compress,
                edge::Meta {
                    key: key::Array::compress(&snapshot.key, *key, &meta.key),
                    kind: snapshot.kind,
                    frozen: false,
                },
                *data,
            ),

            // Grow
            _ if len == <Self as node::Info>::GROW => (
                node::Op::Grow,
                edge::Meta {
                    key: snapshot.key,
                    kind: <<Self as node::Info>::Grow as node::Info>::KIND,
                    frozen: false,
                },
                edge::Data::new_node::<<Self as node::Info>::Grow, _>(edges.into_iter().take(len)),
            ),

            // Replace
            _ => (
                node::Op::Replace,
                edge::Meta {
                    key: snapshot.key,
                    kind: <Self as node::Info>::KIND,
                    frozen: false,
                },
                edge::Data::new_node::<Self, _>(edges.into_iter().take(len)),
            ),
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

enum Reservation<K> {
    Found(usize),
    Reserve(Header<K>, usize),
    Grow,
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

    fn get_or_reserve(&self, key: u8) -> Reservation<K> {
        let index = self.get(key);
        let len = self.len();

        if index < len {
            return Reservation::Found(index);
        }

        if len >= K::LEN {
            return Reservation::Grow;
        }

        Reservation::Reserve(
            Self {
                len: u4::new((len + 1) as u8),
                keys: self.keys.insert(len, key),
                frozen: self.frozen,
            },
            len,
        )
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
