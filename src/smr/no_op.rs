use core::marker::PhantomData;

use crate::byte;
use crate::stat;
use crate::Edge;

#[derive(Default)]
pub(crate) struct Global;

impl Global {
    pub(crate) fn pin(&self) -> Local {
        Local(PhantomData)
    }
}

pub(crate) struct Local<'g>(PhantomData<&'g Global>);

impl<'g> Local<'g> {
    #[inline]
    pub(crate) fn protect<'l>(&'l mut self, _prefix: byte::Array) -> Guard<'g, 'l> {
        Guard(PhantomData)
    }
}

pub(crate) struct Guard<'g, 'l>(PhantomData<&'l mut Local<'g>>);

impl Guard<'_, '_> {
    #[inline]
    pub(crate) unsafe fn retire(&mut self, edge: ribbit::Packed<Edge>) {
        if edge.meta().leaf() || edge.data() == 0 {
            return;
        }

        stat::increment(stat::Counter::Retire);
    }
}
