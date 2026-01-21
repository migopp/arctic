use crate::concurrent::smr;
use crate::concurrent::Smr;

#[derive(Default)]
pub struct NoOp;

impl Smr for NoOp {
    type Local<'g> = Self;

    fn local<'g>(&'g self) -> Self::Local<'g> {
        Self
    }
}

impl smr::Local for NoOp {
    type Guard<'l>
        = Self
    where
        Self: 'l;

    fn guard<'l>(&'l mut self) -> Self::Guard<'l> {
        Self
    }
}

impl smr::Guard for NoOp {
    unsafe fn retire_node<M: ribbit::Pack<Packed: crate::raw::edge::Meta>>(
        &mut self,
        _bits: usize,
        _edge: ribbit::Packed<crate::raw::node::Ptr<M>>,
    ) {
    }

    unsafe fn retire_value<'v, V: crate::sequential::Value<'v>>(&mut self, _value: V::Borrow<'v>) {}
}
