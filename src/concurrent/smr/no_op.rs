use core::marker::PhantomData;

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
        = Guard<(), V>
    where
        V: 'g,
        Self: 'g;

    fn guard<'g>(&'g self, _hazard: ribbit::Packed<P>) -> Self::Guard<'g>
    where
        V: 'g,
    {
        Guard::default()
    }
}

pub struct Guard<G, V> {
    _guard: PhantomData<G>,
    _value: PhantomData<V>,
}

impl<G, V> Default for Guard<G, V> {
    fn default() -> Self {
        Self {
            _guard: PhantomData,
            _value: PhantomData,
        }
    }
}

impl<G, V: Value> smr::Guard<V> for Guard<G, V> {
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

impl<G, V: Value> From<G> for Guard<G, V> {
    #[inline]
    fn from(_: G) -> Self {
        Self::default()
    }
}
