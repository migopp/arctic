use core::fmt::Debug;
use core::sync::atomic::Ordering;

use ribbit::atomic::A128;
use ribbit::atomic::A32;
use ribbit::u2;
use ribbit::u24;
use ribbit::u48;
use ribbit::unpack;

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
    header: A32<Header>,

    _pad: [u32; 3],

    edges: [A128<Edge>; 3],
}

const _: () = assert!(core::mem::size_of::<Node3>() == 64);

impl Node3 {
    pub(crate) fn new() -> Self {
        Self {
            header: A32::new(Header::default()),
            _pad: [0; 3],
            edges: core::array::from_fn(|_| A128::new(Edge::default())),
        }
    }
}

impl Node for Node3 {
    fn get(&self, key: u8) -> Option<&A128<Edge>> {
        let index = self.header.load(Ordering::Acquire).get(key)?;
        Some(&self.edges[index as usize])
    }

    fn get_or_reserve(&self, key: u8) -> Result<&A128<Edge>, Frozen> {
        let mut old = self.header.load(Ordering::Acquire);

        while !old.frozen() {
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

    fn reserve(&mut self, key: u8) -> Option<&mut A128<Edge>> {
        // FIXME: shouldn't need atomics with &mut
        let header = self.header.load(Ordering::Relaxed);
        let (header, index) = header.get_or_reserve(key)?;
        self.header.store(header, Ordering::Relaxed);
        Some(&mut self.edges[index as usize])
    }

    fn is_frozen(&self) -> bool {
        self.header.load(Ordering::Relaxed).frozen()
    }

    fn freeze(&self) {
        let mut old = self.header.load(Ordering::Relaxed);

        while !old.frozen() {
            match self.header.compare_exchange(
                old,
                old.with_frozen(true),
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(conflict) => old = conflict,
            }
        }

        self.edges
            .iter()
            .take(old.len().value() as usize)
            .for_each(Edge::freeze);
    }

    fn replace(&self, snapshot: &Edge) -> (Op, Edge) {
        let header = self.header.load(Ordering::Relaxed);
        let keys = header.keys().value();

        assert!(header.frozen());

        let mut edges: [(u8, Edge); 3] = core::array::from_fn(|_| (0, Edge::default()));
        let mut len = 0;

        self.edges
            .iter()
            .take(header.len().value() as usize)
            .map(|edge| edge.load(Ordering::Relaxed))
            .inspect(|edge| assert!(edge.frozen()))
            .enumerate()
            .filter(|(_, edge)| !matches!(edge.kind().unpack(), <unpack![node::Kind]>::None))
            .map(|(index, edge)| ((keys >> (index * 8)) as u8, edge.with_frozen(false)))
            .zip(&mut edges)
            .for_each(|(edge, save)| {
                *save = edge;
                len += 1;
            });

        let edges = &edges[..len];

        match edges {
            [] => (
                Op::Destroy,
                snapshot
                    .with_key(key::Array::default())
                    .with_kind(node::Kind::new(<unpack![node::Kind]>::None)),
            ),

            [(key, child)] if key::Array::can_compress(&snapshot.key(), &child.key()) => (
                Op::Compress,
                Edge::new(
                    key::Array::compress(&snapshot.key(), *key, &child.key()),
                    false,
                    child.kind(),
                    child.next(),
                ),
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
                    snapshot
                        .with_kind(node::Kind::new(<unpack![node::Kind]>::Node256))
                        .with_next(u48::new(node as u64)),
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
                    snapshot
                        .with_kind(node::Kind::new(<unpack![node::Kind]>::Node3))
                        .with_next(u48::new(node as u64)),
                )
            }
        }
    }
}

#[ribbit::pack(size = 32, debug)]
struct Header {
    len: u2,
    frozen: bool,
    #[ribbit(offset = 8, debug(format = "{:#08x}"))]
    keys: u24,
}

impl Default for Header {
    fn default() -> Self {
        Self::new(u2::new(0), false, u24::new(0))
    }
}

impl Header {
    fn get(&self, key: u8) -> Option<u8> {
        let keys = self.keys().value();
        let len = self.len().value();
        (0..len).find(|i| (keys >> (i * 8)) as u8 == key)
    }

    fn get_or_reserve(&self, key: u8) -> Option<(Self, u8)> {
        if let Some(index) = self.get(key) {
            return Some((*self, index));
        }

        let keys = self.keys().value();
        let len = self.len().value();
        match len {
            0..3 => Some((
                self.with_len(u2::new(len + 1))
                    .with_keys(u24::new(keys | ((key as u32) << (len * 8)))),
                len,
            )),
            _ => None,
        }
    }
}
