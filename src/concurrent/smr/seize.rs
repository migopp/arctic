use crate::concurrent::smr;
use crate::concurrent::Smr;
use crate::concurrent::Value;
use crate::raw::node;
use crate::stat;

use seize::Guard as _;

#[derive(Default)]
pub struct Seize(seize::Collector);

impl<P: ribbit::Pack<Packed: smr::hazard::Prefix>, V: Value> Smr<P, V> for Seize {
    // In this case, there is no notion of a local handle.
    // We get the same effect from calling `enter` on a `Collector`.
    //
    // So, just forward `Self`.
    type Local<'g> = Local<'g>;

    fn local<'g>(&'g self) -> Self::Local<'g> {
        Local(&self.0)
    }
}

pub struct Local<'g>(&'g seize::Collector);

impl<'g, P: ribbit::Pack<Packed: smr::hazard::Prefix>, V: Value> smr::Local<P, V> for Local<'g> {
    type Guard<'l>
        = Guard<'l>
    where
        'g: 'l;

    fn guard<'l>(&'l mut self, _hazad: ribbit::Packed<P>) -> Self::Guard<'l> {
        Guard(self.0.enter())
    }
}

pub struct Guard<'g>(seize::LocalGuard<'g>);

impl<'g, V: Value> smr::Guard<V> for Guard<'g> {
    #[expect(private_bounds)]
    #[expect(private_interfaces)]
    unsafe fn retire_node<M: ribbit::Pack<Packed: crate::raw::edge::Meta>>(
        &mut self,
        _bits: usize,
        node: ribbit::Packed<node::Ptr<M>>,
    ) {
        use std::num::NonZeroU64;

        // HACK: Unfortunately, Seize does not natively support `defer_unchecked`.
        // However, `defer_retire` does take an arbitrary closure to run at retire-time,
        // and passes the `ptr` argument directly to it...
        //
        // See: [`seize::raw::Collector::add`] and [`seize::raw::Collector::try_retire`].
        self.0.defer_retire(
            node.raw().get() as *mut ribbit::Packed<node::Ptr<M>>,
            |node_as_ptr, _| {
                let node_raw_contents = NonZeroU64::new_unchecked(node_as_ptr as u64);
                let node = <node::Ptr<M> as ribbit::Pack>::Packed::new_unchecked(node_raw_contents);
                node.deallocate(stat::Counter::FreeRetire);
            },
        );
    }

    unsafe fn retire_value(&mut self, value: u64) {
        // HACK: Same trick as above.
        self.0.defer_retire(value as *mut u64, |value_as_ptr, _| {
            let value = value_as_ptr as u64;
            drop(V::from_raw(value))
        });
    }
}
