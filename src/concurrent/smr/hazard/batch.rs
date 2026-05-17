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
