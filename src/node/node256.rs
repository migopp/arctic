use core::sync::atomic::Ordering;

use ribbit::atomic::A128;

use crate::node::Edge;
use crate::node::Frozen;
use crate::node::Op;
use crate::Node;

#[repr(C)]
#[derive(Debug)]
pub(crate) struct Node256([A128<Edge>; 256]);

impl Node256 {
    pub(crate) fn new() -> Self {
        Self(core::array::from_fn(|_| A128::new(Edge::default())))
    }
}

impl Node for Node256 {
    fn get(&self, key: u8) -> Option<&A128<Edge>> {
        Some(&self.0[key as usize])
    }

    fn get_or_reserve(&self, key: u8) -> Result<&A128<Edge>, Frozen> {
        Ok(&self.0[key as usize])
    }

    fn reserve(&mut self, key: u8) -> Option<&mut A128<Edge>> {
        Some(&mut self.0[key as usize])
    }

    fn is_frozen(&self) -> bool {
        self.0[0].load(Ordering::Relaxed).frozen()
    }

    fn freeze(&self) {
        self.0.iter().for_each(Edge::freeze);
    }

    fn replace(&self, _snapshot: &Edge) -> (Op, Edge) {
        todo!()
    }
}
