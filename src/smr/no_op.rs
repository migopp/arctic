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
    pub(crate) fn protect_read<'l>(
        &'l self,
        _prefix: ribbit::Packed<byte::Array>,
    ) -> ReadGuard<'g, 'l> {
        ReadGuard(PhantomData)
    }

    #[inline]
    pub(crate) fn protect_write<'l>(
        &'l mut self,
        _prefix: ribbit::Packed<byte::Array>,
    ) -> WriteGuard<'g, 'l> {
        WriteGuard(PhantomData)
    }
}

pub(crate) struct ReadGuard<'g, 'l>(PhantomData<&'l Local<'g>>);

pub(crate) struct WriteGuard<'g, 'l>(PhantomData<&'l mut Local<'g>>);

impl WriteGuard<'_, '_> {
    #[inline]
    pub(crate) unsafe fn retire(&mut self, edge: ribbit::Packed<Edge>) {
        if edge.meta().leaf() || edge.data() == 0 {
            return;
        }

        stat::increment(stat::Counter::Retire);
    }
}
