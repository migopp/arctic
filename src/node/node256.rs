use core::marker::PhantomData;
use core::ptr::NonNull;

use ribbit::atomic::Atomic128;

use crate::edge;
use crate::node;
use crate::node::Edge;
use crate::node::Node15;
use crate::node::Op;
use crate::Node;

#[repr(C, align(4096))]
#[derive(Debug)]
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

    fn replace(&self, _parent: ribbit::Packed<edge::Meta>) -> (Op, ribbit::Packed<Edge>) {
        todo!()
    }
}

impl Node256 {
    #[inline]
    pub(crate) fn iter_range(&self, min: Option<u8>, max: Option<u8>) -> Iter {
        Iter::new(min, max, &self.0)
    }
}

impl<'a> IntoIterator for &'a Node256 {
    type Item = (u8, &'a Atomic128<Edge>);
    type IntoIter = Iter<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        Iter::new(None, None, &self.0)
    }
}

impl node::Info for Node256 {
    const KIND: node::Kind = node::Kind::Node256;
    const GROW: usize = 256;
    const REF: for<'a> fn(&'a Self) -> node::Ref<'a> = |node| node::Ref::Node256(node);
    type Grow = Node256;
    type Shrink = Node15;
}

#[derive(Copy, Clone)]
pub(crate) struct Iter<'a> {
    head: u16,
    tail: u16,
    min: Option<u8>,
    max: Option<u8>,
    edges: NonNull<Atomic128<Edge>>,
    _slice: PhantomData<&'a [Atomic128<Edge>]>,
}

impl<'a> Iter<'a> {
    #[inline]
    fn new(min: Option<u8>, max: Option<u8>, edges: &'a [Atomic128<Edge>]) -> Self {
        Self {
            head: min.unwrap_or(0) as u16,
            tail: max.unwrap_or(255) as u16 + 1,
            min,
            max,
            edges: NonNull::from(edges).cast(),
            _slice: PhantomData,
        }
    }

    #[inline]
    pub(crate) fn min(&self) -> Option<u8> {
        self.min
    }

    #[inline]
    pub(crate) fn max(&self) -> Option<u8> {
        self.max
    }
}

impl<'a> Iterator for Iter<'a> {
    type Item = (u8, &'a Atomic128<Edge>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.head == self.tail {
            return None;
        }

        let next = (self.head as u8, unsafe {
            self.edges.add(self.head as usize).as_ref()
        });
        self.head += 1;
        Some(next)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = (self.tail - self.head) as usize;
        (len, Some(len))
    }
}

impl<'a> ExactSizeIterator for Iter<'a> {
    #[inline]
    fn len(&self) -> usize {
        let (lower, upper) = self.size_hint();
        validate_eq!(upper, Some(lower));
        lower
    }
}

impl<'a> DoubleEndedIterator for Iter<'a> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.head == self.tail {
            return None;
        }

        self.tail -= 1;
        let next = unsafe { self.edges.add(self.tail as usize).as_ref() };
        Some((self.tail as u8, next))
    }
}
