use crate::concurrent::smr;
use crate::concurrent::Smr;
use crate::concurrent::Value;
use crate::raw::node;
use crate::stat;

#[derive(Default)]
pub struct Epoch(crossbeam_epoch::Collector);

impl<P: ribbit::Pack<Packed: smr::hazard::Prefix>, V: Value> Smr<P, V> for Epoch {
    type Local<'g> = Local;

    fn local<'g>(&'g self) -> Self::Local<'g> {
        Local(self.0.register())
    }
}

pub struct Local(crossbeam_epoch::LocalHandle);

impl<P: ribbit::Pack<Packed: smr::hazard::Prefix>, V: Value> smr::Local<P, V> for Local {
    type Guard<'l>
        = Guard
    where
        Self: 'l;

    fn guard<'l>(&'l mut self, _hazard: ribbit::Packed<P>) -> Self::Guard<'l> {
        Guard(self.0.pin())
    }
}

pub struct Guard(crossbeam_epoch::Guard);

impl<V: Value> smr::Guard<V> for Guard {
    #[expect(private_bounds)]
    #[expect(private_interfaces)]
    unsafe fn retire_node<M: ribbit::Pack<Packed: crate::raw::edge::Meta>>(
        &mut self,
        _bits: usize,
        node: ribbit::Packed<node::Ptr<M>>,
    ) {
        self.0.defer_unchecked(move || {
            node.deallocate(stat::Counter::FreeRetire);
        });
    }

    unsafe fn retire_value(&mut self, value: u64) {
        self.0.defer_unchecked(move || drop(V::from_raw(value)));
    }
}
