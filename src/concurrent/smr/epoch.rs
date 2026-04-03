use core::cell::UnsafeCell;

use crossbeam_epoch::LocalHandle;

use crate::concurrent::Smr;
use crate::concurrent::Value;
use crate::concurrent::smr;
use crate::raw::node;
use crate::stat;

pub struct Epoch {
    collector: crossbeam_epoch::Collector,
    locals: [UnsafeCell<Option<LocalHandle>>; smr::thread::MAX],
}

impl Default for Epoch {
    fn default() -> Self {
        Self {
            collector: crossbeam_epoch::Collector::default(),
            locals: core::array::from_fn(|_| UnsafeCell::new(None)),
        }
    }
}

impl Epoch {
    pub fn with_bag_capacity(max_objects: usize) -> Self {
        crossbeam_epoch::set_bag_capacity(max_objects);
        Self::default()
    }

    fn local(&self) -> &LocalHandle {
        let id = smr::thread::Id::current();
        let local = &self.locals[usize::from(id)];
        match unsafe { local.get().as_ref().unwrap() } {
            Some(local) => local,
            None => self.local_cold(),
        }
    }

    #[cold]
    fn local_cold(&self) -> &LocalHandle {
        let id = smr::thread::Id::current();
        let local = &self.locals[usize::from(id)];
        unsafe { local.get().as_mut().unwrap() }.insert(self.collector.register())
    }
}

impl<P: ribbit::Pack<Packed: smr::hazard::Prefix>, V: Value> Smr<P, V> for Epoch {
    type Local<'g> = &'g LocalHandle;

    fn local<'g>(&'g self) -> Self::Local<'g> {
        self.local()
    }
}

impl<P: ribbit::Pack<Packed: smr::hazard::Prefix>, V: Value> smr::Local<P, V> for &'_ LocalHandle {
    type Guard<'l>
        = Guard
    where
        Self: 'l;

    fn guard<'l>(&'l mut self, _hazard: ribbit::Packed<P>) -> Self::Guard<'l> {
        Guard(self.pin())
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
        stat::increment(stat::Counter::Retire);

        unsafe {
            self.0.defer_unchecked(move || {
                node.deallocate(stat::Counter::FreeRetire);
            });
        }
    }

    unsafe fn retire_value(&mut self, value: u64) {
        stat::increment(stat::Counter::Retire);

        unsafe {
            self.0.defer_unchecked(move || drop(V::from_raw(value)));
        }
    }
}
