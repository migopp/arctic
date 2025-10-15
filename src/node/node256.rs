use core::fmt::Debug;

use ribbit::atomic::Atomic128;

use crate::edge;
use crate::node;
use crate::node::Edge;
use crate::node::Node15;
use crate::node::Op;
use crate::Node;

#[repr(C, align(4096))]
pub(crate) struct Node256([Atomic128<Edge>; 256]);

const _: () = assert!(core::mem::size_of::<Node256>() == 4096);
const _: () = assert!(core::mem::align_of::<Node256>() == 4096);

impl Default for Node256 {
    fn default() -> Self {
        Self(core::array::from_fn(|_| Atomic128::default()))
    }
}

impl Node for Node256 {
    #[inline]
    fn edges(&self) -> &[Atomic128<Edge>] {
        &self.0
    }

    #[inline]
    fn get(&self, key: u8) -> Option<&Atomic128<Edge>> {
        // SAFETY: `key` is a u8 and must be < 256
        Some(unsafe { self.0.get_unchecked(key as usize) })
    }

    #[inline]
    fn get_or_reserve(&self, key: u8) -> Option<&Atomic128<Edge>> {
        self.get(key)
    }

    #[inline]
    fn reserve(&mut self, key: u8) -> Option<&mut Atomic128<Edge>> {
        // SAFETY: `key` is a u8 and must be < 256
        Some(unsafe { self.0.get_unchecked_mut(key as usize) })
    }

    fn replace(&self, _parent: ribbit::Packed<Edge>) -> (Op, ribbit::Packed<Edge>) {
        todo!()
    }
}

impl Node256 {
    #[inline]
    pub(crate) fn keys_sorted(&self) -> KeyIter {
        KeyIter::new(None, None)
    }

    #[inline]
    pub(crate) fn keys_range(&self, min: Option<u8>, max: Option<u8>) -> KeyIter {
        KeyIter::new(min, max)
    }
}

impl Debug for Node256 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Node256")
            .field("edges", &edge::DebugSlice(&self.0))
            .finish()
    }
}

impl node::Info for Node256 {
    const KIND: node::Kind = node::Kind::Node256;
    const GROW: usize = 256;
    const REF: for<'a> fn(&'a Self) -> node::Ref<'a> = |node| node::Ref::Node256(node);
    type Grow = Node256;
    type Shrink = Node15;
}

#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct KeyIter {
    head: u16,
    tail: u16,

    // FIXME: handle big-endian
    _discriminant: Discriminant,
}

#[repr(u32)]
#[derive(Copy, Clone)]
enum Discriminant {
    Node256 = 1u32.rotate_right(1),
}

impl KeyIter {
    #[inline]
    fn new(min: Option<u8>, max: Option<u8>) -> Self {
        Self {
            head: min.unwrap_or(0) as u16,
            tail: max.unwrap_or(255) as u16 + 1,
            _discriminant: Discriminant::Node256,
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
