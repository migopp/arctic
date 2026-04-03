pub mod epoch;
pub mod hazard;
mod no_op;
pub mod seize;
mod thread;

pub use epoch::Epoch;
pub use hazard::Hazard;
pub use no_op::NoOp;
pub use seize::Seize;

use crate::concurrent::Value;
use crate::raw::edge;
use crate::raw::node;
use hazard::Prefix;

pub trait Smr {
    type Global<P, V>: Global<P, V>
    where
        P: ribbit::Pack<Packed: Prefix>,
        V: Value;
}

pub trait Global<P: ribbit::Pack<Packed: Prefix>, V: Value>: Default {
    type Guard<'g>: Guard<V>
    where
        V: 'g,
        Self: 'g;

    fn guard<'g>(&'g self, hazard: ribbit::Packed<P>) -> Self::Guard<'g>
    where
        V: 'g;
}

pub trait Guard<V: Value> {
    #[expect(private_bounds)]
    #[expect(private_interfaces)]
    unsafe fn retire_node<M: ribbit::Pack<Packed: edge::Meta>>(
        &mut self,
        bits: usize,
        edge: ribbit::Packed<node::Ptr<M>>,
    );

    unsafe fn retire_value(&mut self, raw: u64);
}
