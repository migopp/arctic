use crate::concurrent::smr;
use crate::concurrent::Smr;
use crate::raw::edge;
use crate::stat;

#[derive(Default)]
pub struct Epoch(crossbeam_epoch::Collector);

impl Smr for Epoch {
    type Local<'g> = Local;

    fn local<'g>(&'g self) -> Self::Local<'g> {
        Local(self.0.register())
    }
}

pub struct Local(crossbeam_epoch::LocalHandle);

impl smr::Local for Local {
    type Guard<'l>
        = Guard
    where
        Self: 'l;

    fn guard<'l>(&'l mut self) -> Self::Guard<'l> {
        Guard(self.0.pin())
    }
}

pub struct Guard(crossbeam_epoch::Guard);

impl smr::Guard for Guard {
    unsafe fn retire_node<M: ribbit::Pack<Packed: crate::raw::edge::Meta>>(
        &mut self,
        _bits: usize,
        edge: ribbit::Packed<crate::raw::Edge<M>>,
    ) {
        self.0.defer_unchecked(move || match edge.child() {
            None | Some(edge::Child::Value(_)) => unreachable!(),
            Some(edge::Child::Node(node)) => {
                node.deallocate(stat::Counter::FreeRetire);
            }
        });
    }

    unsafe fn retire_value<'v, V: crate::sequential::Value<'v>>(&mut self, value: V::Borrow<'v>) {
        let raw = V::borrow_into_raw(value);
        self.0.defer_unchecked(move || drop(V::from_raw(raw)));
    }
}
