use core::marker::PhantomData;

use crate::concurrent::Value;
use crate::concurrent::smr::hazard;
use crate::concurrent::smr::hazard::prefix::Prefix;
use crate::stat;

pub struct EpochBatch<P: ribbit::Pack<Packed: Prefix>, V: Value> {
    pub batch: Vec<(ribbit::Packed<P>, u64)>,
    pub epoch: usize,
    _value: PhantomData<V>,
}

impl<P: ribbit::Pack<Packed: Prefix>, V: Value> EpochBatch<P, V> {
    pub fn new(epoch: usize, sz_hint: usize) -> Self {
        Self {
            batch: Vec::with_capacity(sz_hint),
            epoch,
            _value: PhantomData,
        }
    }

    #[inline]
    pub fn push(&mut self, retiree: (ribbit::Packed<P>, u64)) {
        self.batch.push(retiree)
    }

    pub fn deallocate(&mut self) {
        self.batch.drain(..).for_each(|(prefix, raw)| {
            hazard::deallocate::<P, V>(prefix, raw, stat::Counter::FreeReclaim)
        });
    }

    pub fn drain_last(&mut self, n: usize) -> std::vec::Drain<'_, (ribbit::Packed<P>, u64)> {
        let split = self.batch.len().saturating_sub(n);
        self.batch.drain(split..)
    }
}
