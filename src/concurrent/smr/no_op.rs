use crate::concurrent::smr;
use crate::concurrent::Smr;
use crate::concurrent::Value;

#[derive(Default)]
pub struct NoOp;

impl<'v, P: ribbit::Pack<Packed: smr::hazard::Prefix>, V: Value<'v>> Smr<'v, P, V> for NoOp {
    type Local<'g> = Self;

    fn local<'g>(&'g self) -> Self::Local<'g> {
        Self
    }
}

impl<'v, P: ribbit::Pack<Packed: smr::hazard::Prefix>, V: Value<'v>> smr::Local<'v, P, V> for NoOp {
    type Guard<'l>
        = Self
    where
        Self: 'l;

    fn guard<'l>(&'l mut self, _hazard: ribbit::Packed<P>) -> Self::Guard<'l> {
        Self
    }
}

impl<'v, V: Value<'v>> smr::Guard<'v, V> for NoOp {
    unsafe fn retire_node<M: ribbit::Pack<Packed: crate::raw::edge::Meta>>(
        &mut self,
        _bits: usize,
        _edge: ribbit::Packed<crate::raw::node::Ptr<M>>,
    ) {
    }

    unsafe fn retire_value(&mut self, _value: V::Borrow<'v>) {}
}
