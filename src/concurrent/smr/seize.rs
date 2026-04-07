use crate::concurrent::Smr;
use crate::concurrent::Value;
use crate::concurrent::smr;
use crate::concurrent::smr::Prefix;
use crate::raw::node;
use crate::stat;

use seize::Guard as _;

pub struct Seize;

impl Smr for Seize {
    type Global<P, V>
        = Global
    where
        P: ribbit::Pack<Packed: smr::hazard::Prefix>,
        V: Value;
}

#[derive(Default)]
pub struct Global(seize::Collector);

impl Global {
    pub fn with_batch_size(batch_size: usize) -> Self {
        Self(seize::Collector::new().batch_size(batch_size))
    }
}

impl<P: ribbit::Pack<Packed: Prefix>, V: Value> smr::Global<P, V> for Global {
    type Guard<'g>
        = seize::LocalGuard<'g>
    where
        V: 'g,
        Self: 'g;

    fn guard<'g>(&'g self, _hazard: ribbit::Packed<P>) -> Self::Guard<'g>
    where
        V: 'g,
    {
        self.0.enter()
    }
}

impl<'g, V: Value> smr::Guard<V> for seize::LocalGuard<'g> {
    #[expect(private_bounds)]
    #[expect(private_interfaces)]
    unsafe fn retire_node<M: ribbit::Pack<Packed: crate::raw::edge::Meta>>(
        &mut self,
        _bits: usize,
        node: ribbit::Packed<node::Ptr<M>>,
    ) {
        stat::increment(stat::Counter::Retire);

        unsafe {
            self.defer_retire(node.raw().get() as *mut (), |ptr, _| {
                node::Ptr::<M>::new_unchecked(ptr as u64).deallocate(stat::Counter::FreeRetire)
            })
        }
    }

    unsafe fn retire_value(&mut self, value: u64) {
        stat::increment(stat::Counter::Retire);

        // HACK: Unfortunately, Seize does not natively support `defer_unchecked`.
        // However, `defer_retire` does take an arbitrary closure to run at retire-time,
        // and passes the `ptr` argument directly to it...
        //
        // See: [`seize::raw::Collector::add`] and [`seize::raw::Collector::try_retire`].
        //
        unsafe {
            self.defer_retire(value as *mut (), |ptr, _| {
                stat::increment(stat::Counter::FreeRetire);
                drop(V::from_raw(ptr as u64))
            });
        }
    }
}
