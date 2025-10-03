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

impl<'a> IntoIterator for &'a Node256 {
    type Item = (u8, &'a Atomic128<Edge>);
    type IntoIter = Iter<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        Iter {
            key: 0,
            edges: self.0.iter(),
        }
    }
}

impl node::Info for Node256 {
    const KIND: node::Kind = node::Kind::Node256;
    const GROW: usize = 256;
    const REF: for<'a> fn(&'a Self) -> node::Ref<'a> = |node| node::Ref::Node256(node);
    type Grow = Node256;
    type Shrink = Node15;
}

pub(crate) struct Iter<'a> {
    key: u8,
    edges: core::slice::Iter<'a, Atomic128<Edge>>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = (u8, &'a Atomic128<Edge>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let edge = self.edges.next()?;
        let key = self.key;
        self.key = self.key.wrapping_add(1);
        Some((key, edge))
    }
}
