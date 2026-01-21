pub mod epoch;
mod no_op;

pub use epoch::Epoch;
pub use no_op::NoOp;

use crate::raw::edge;
use crate::raw::Edge;

pub trait Smr: Default {
    type Local<'g>: Local
    where
        Self: 'g;

    fn local<'g>(&'g self) -> Self::Local<'g>;
}

pub trait Local {
    type Guard<'l>: Guard
    where
        Self: 'l;

    fn guard<'l>(&'l mut self) -> Self::Guard<'l>;
}

pub trait Guard {
    unsafe fn retire_node<M: ribbit::Pack<Packed: edge::Meta>>(
        &mut self,
        bits: usize,
        edge: ribbit::Packed<Edge<M>>,
    );

    unsafe fn retire_value<'v, V: crate::sequential::Value<'v>>(&mut self, value: V::Borrow<'v>);
}
