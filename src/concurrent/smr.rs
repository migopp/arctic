pub mod epoch;
pub mod hazard;
mod no_op;
pub mod seize;

pub use epoch::Epoch;
pub use hazard::Hazard;
pub use no_op::NoOp;
pub use seize::Seize;

use crate::concurrent::Value;
use crate::raw::edge;
use crate::raw::node;
use hazard::Prefix;

pub trait Smr<P: ribbit::Pack<Packed: Prefix>, V: Value>: Default {
    type Local<'g>: Local<P, V>
    where
        Self: 'g;

    fn local<'g>(&'g self) -> Self::Local<'g>;
}

pub trait Local<P: ribbit::Pack<Packed: Prefix>, V: Value> {
    type Guard<'l>: Guard<V>
    where
        Self: 'l;

    fn guard<'l>(&'l mut self, hazard: ribbit::Packed<P>) -> Self::Guard<'l>;
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
