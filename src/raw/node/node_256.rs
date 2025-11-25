use core::fmt::Debug;

use ribbit::Atomic;

use crate::raw::edge;
use crate::raw::node;
use crate::raw::node::Edge;
use crate::raw::node::Node47;
use crate::raw::Node;

#[repr(C, align(4096))]
pub(crate) struct Node256<M: ribbit::Pack>([Atomic<Edge<M>>; 256]);

const _: () = assert!(core::mem::size_of::<Node256<()>>() == 4096);
const _: () = assert!(core::mem::align_of::<Node256<()>>() == 4096);

impl<M> Default for Node256<M>
where
    M: ribbit::Pack<Packed: edge::Meta>,
{
    fn default() -> Self {
        Self(core::array::from_fn(|_| Atomic::new_packed(Edge::DEFAULT)))
    }
}

impl<M> Node<M> for Node256<M>
where
    M: ribbit::Pack<Packed: edge::Meta>,
{
    const KIND: node::Kind = node::Kind::Node256;
    const LEN: usize = 256;

    type Grow = Node256<M>;
    type Shrink = Node47<M>;

    #[inline]
    fn keys<L: node::iter::Lower, U: node::iter::Upper>(
        &self,
        lower: L,
        upper: U,
    ) -> node::KeyIter {
        node::KeyIter::from_node_256(KeyIter::new(lower, upper))
    }

    #[inline]
    fn edges(&self) -> &[Atomic<Edge<M>>] {
        &self.0
    }

    #[inline]
    fn get(&self, key: u8) -> Option<&Atomic<Edge<M>>> {
        // SAFETY: `key` is a u8 and must be < 256
        Some(unsafe { self.0.get_unchecked(key as usize) })
    }

    #[inline]
    fn get_or_insert(&self, key: u8) -> Option<&Atomic<Edge<M>>> {
        self.get(key)
    }

    #[inline]
    fn insert(&mut self, key: u8) -> Option<&mut Atomic<Edge<M>>> {
        // SAFETY: `key` is a u8 and must be < 256
        Some(unsafe { self.0.get_unchecked_mut(key as usize) })
    }

    fn freeze(&self) {
        self.0.iter().for_each(Edge::freeze);
    }
}

impl<M> Debug for Node256<M>
where
    M: ribbit::Pack<Packed: edge::Meta + Debug>,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Node256").field("edges", &self.0).finish()
    }
}

#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct KeyIter {
    head: u16,
    tail: u16,

    // FIXME: handle big-endian
    _tag: Tag,
}

#[repr(u32)]
#[derive(Copy, Clone)]
enum Tag {
    Node256 = (node::Kind::Node256 as u32) << 30,
}

impl KeyIter {
    #[inline]
    fn new<L: node::iter::Lower, U: node::iter::Upper>(lower: L, upper: U) -> Self {
        Self {
            head: lower.get() as u16,
            tail: upper.get() as u16 + 1,
            _tag: Tag::Node256,
        }
    }
}

impl Iterator for KeyIter {
    type Item = u8;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.head == self.tail {
            return None;
        }

        let next = self.head as u8;
        self.head += 1;
        Some(next)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = (self.tail - self.head) as usize;
        (len, Some(len))
    }
}

impl ExactSizeIterator for KeyIter {
    #[inline]
    fn len(&self) -> usize {
        let (lower, upper) = self.size_hint();
        validate_eq!(upper, Some(lower));
        lower
    }
}

impl DoubleEndedIterator for KeyIter {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.head == self.tail {
            return None;
        }

        self.tail -= 1;
        Some(self.tail as u8)
    }
}
