use crate::concurrent::smr;
use crate::concurrent::Smr;
use crate::concurrent::Value;

#[derive(Default)]
pub struct NoOp;

impl<P: ribbit::Pack<Packed: smr::hazard::Prefix>, V: Value> Smr<P, V> for NoOp {
    type Local<'g> = Self;

    fn local<'g>(&'g self) -> Self::Local<'g> {
        Self
    }
}

impl<P: ribbit::Pack<Packed: smr::hazard::Prefix>, V: Value> smr::Local<P, V> for NoOp {
    type Guard<'l>
        = Self
    where
        Self: 'l;

    fn guard<'l>(&'l mut self, _hazard: ribbit::Packed<P>) -> Self::Guard<'l> {
        Self
    }
}

impl<V: Value> smr::Guard<V> for NoOp {
    #[expect(private_bounds)]
    #[expect(private_interfaces)]
    unsafe fn retire_node<M: ribbit::Pack<Packed: crate::raw::edge::Meta>>(
        &mut self,
        _bits: usize,
        _edge: ribbit::Packed<crate::raw::node::Ptr<M>>,
    ) {
    }

    unsafe fn retire_value(&mut self, _value: u64) {}
}
