use core::sync::atomic::Ordering;

use crate::edge;
use crate::node;
use crate::node::Edge;
use crate::node::Frozen;
use crate::node::Node15;
use crate::node::Op;
use crate::Node;

#[repr(transparent)]
#[derive(Debug)]
pub(crate) struct Node256([Edge; 256]);

const _: () = assert!(core::mem::size_of::<Node256>() == 4096);

impl Default for Node256 {
    fn default() -> Self {
        Self(core::array::from_fn(|_| Edge::default()))
    }
}

impl Node for Node256 {
    fn get(&self, key: u8) -> Option<&Edge> {
        Some(&self.0[key as usize])
    }

    fn get_or_reserve(&self, key: u8) -> Result<&Edge, Frozen> {
        Ok(&self.0[key as usize])
    }

    fn reserve(&mut self, key: u8) -> Option<&mut Edge> {
        Some(&mut self.0[key as usize])
    }

    fn is_frozen(&self) -> bool {
        self.0[0].load_low_packed(Ordering::Relaxed).frozen()
    }

    fn freeze(&self) {
        self.0.iter().for_each(Edge::freeze);
    }

    fn replace(&self, _snapshot: &edge::Meta) -> (Op, edge::Meta, edge::Data) {
        todo!()
    }
}

impl<'a> IntoIterator for &'a Node256 {
    type Item = (u8, &'a Edge);
    type IntoIter = node::Iter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        super::KeyIter::new_256().zip(super::EdgeIter::new(&self.0))
    }
}

impl node::Info for Node256 {
    const KIND: node::Kind = node::Kind::Node256;
    const GROW: usize = 256;
    type Grow = Node256;
    type Shrink = Node15;
}
