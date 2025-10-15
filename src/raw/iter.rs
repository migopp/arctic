pub(crate) mod leaf;
pub(crate) mod postorder;
mod range;

pub(crate) use leaf::LeafIter;
pub(crate) use postorder::PostorderIter;
pub(crate) use range::RangeIter;

// use ribbit::atomic::Atomic128;
//
// use crate::smr;
// use crate::Edge;
//
// pub(crate) struct PessimisticIter<'g, 'l, R, W> {
//     root: &'g Atomic128<Edge>,
//     guard: smr::Guard<'g, 'l>,
//     lock: bool,
//     iter: RangeIter<'g, R, W>,
// }
//
// impl PessimisticIter<'g, 'l, R, W> {}
