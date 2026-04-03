use crate::concurrent::Smr;
use crate::concurrent::Value;
use crate::concurrent::smr;

#[derive(Default)]
pub struct NoOp;

impl Smr for NoOp {
    type Global<P, V>
        = Self
    where
        P: ribbit::Pack<Packed: smr::hazard::Prefix>,
        V: Value;
}

impl<P: ribbit::Pack<Packed: smr::hazard::Prefix>, V: Value> smr::Global<P, V> for NoOp {
    type Guard<'g>
        = Self
    where
        V: 'g,
        Self: 'g;

    fn guard<'g>(&'g self, _hazard: ribbit::Packed<P>) -> Self::Guard<'g>
    where
        V: 'g,
    {
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
