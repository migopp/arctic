use core::marker::PhantomData;

use crate::concurrent::Value;
use crate::concurrent::smr::hazard;
use crate::concurrent::smr::hazard::prefix::Prefix;
use crate::stat;

pub struct Batch<P: ribbit::Pack<Packed: Prefix>, V: Value> {
    pub batch: Vec<(ribbit::Packed<P>, u64)>,
    _value: PhantomData<V>,
}

impl<P: ribbit::Pack<Packed: Prefix>, V: Value> Batch<P, V> {
    pub fn new(batch: Vec<(ribbit::Packed<P>, u64)>) -> Self {
        Self {
            batch,
            _value: PhantomData,
        }
    }

    pub fn deallocate(&mut self) {
        self.batch.drain(..).for_each(|(prefix, raw)| {
            hazard::deallocate::<P, V>(prefix, raw, stat::Counter::FreeReclaim)
        });
    }
}

// FIXME: figure out if there a way to parameterize based on being collectable into a vector?
// Could use `Into<Vec<...>>`, but that is not implemented for `std::vec::Drain`...
impl<'a, P: ribbit::Pack<Packed: Prefix>, V: Value>
    From<std::vec::Drain<'a, (ribbit::Packed<P>, u64)>> for Batch<P, V>
{
    fn from(batch: std::vec::Drain<'a, (ribbit::Packed<P>, u64)>) -> Self {
        Self::new(batch.collect())
    }
}
