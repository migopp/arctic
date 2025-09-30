use core::cell::RefCell;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic64;
use thread_local::ThreadLocal;

use crate::byte;
use crate::node;
use crate::smr::membarrier;
use crate::stat;
use crate::Edge;

const RETIRED_COUNT: usize = 16;

#[repr(C, align(64))]
#[derive(Default)]
struct Cache<T>(T);

pub(crate) struct Global {
    hazards: ThreadLocal<Cache<Atomic64<byte::Array>>>,
    edges: ThreadLocal<Cache<RefCell<Vec<ribbit::Packed<Edge>>>>>,
}

impl Global {
    pub(crate) fn pin(&self) -> Local {
        Local {
            hazards: &self.hazards,
            hazard: &self.hazards.get_or_default().0,
            edges: self.edges.get_or_default().0.borrow_mut(),
        }
    }
}

impl Default for Global {
    fn default() -> Self {
        Self {
            hazards: ThreadLocal::with_capacity(128),
            edges: ThreadLocal::with_capacity(128),
        }
    }
}

impl Drop for Global {
    fn drop(&mut self) {
        self.edges
            .iter_mut()
            .map(|Cache(retired)| retired)
            .flat_map(RefCell::get_mut)
            .for_each(|edge| unsafe { Edge::deallocate(*edge, stat::Counter::FreeDrop) })
    }
}

pub(crate) struct Local<'g> {
    hazards: &'g ThreadLocal<Cache<Atomic64<byte::Array>>>,
    hazard: &'g Atomic64<byte::Array>,
    edges: std::cell::RefMut<'g, Vec<ribbit::Packed<Edge>>>,
}

impl<'g> Local<'g> {
    #[inline]
    pub(crate) fn protect_read<'l>(
        &'l self,
        prefix: ribbit::Packed<byte::Array>,
    ) -> ReadGuard<'g, 'l> {
        self.protect(prefix);
        ReadGuard(self)
    }

    #[inline]
    pub(crate) fn protect_write<'l>(
        &'l mut self,
        prefix: ribbit::Packed<byte::Array>,
    ) -> WriteGuard<'g, 'l> {
        self.protect(prefix);
        WriteGuard(self)
    }

    #[inline]
    fn protect(&self, prefix: ribbit::Packed<byte::Array>) {
        self.hazard.store_packed(prefix, Ordering::Relaxed);
        membarrier::fast();
    }
}

pub(crate) struct ReadGuard<'g, 'l>(&'l Local<'g>);

impl Drop for ReadGuard<'_, '_> {
    #[inline]
    fn drop(&mut self) {
        self.0
            .hazard
            .store_packed(byte::Array::EMPTY, Ordering::Relaxed);
    }
}

pub(crate) struct WriteGuard<'g, 'l>(&'l mut Local<'g>);

impl Drop for WriteGuard<'_, '_> {
    #[inline]
    fn drop(&mut self) {
        self.0
            .hazard
            .store_packed(byte::Array::EMPTY, Ordering::Relaxed);
    }
}

impl WriteGuard<'_, '_> {
    pub(crate) unsafe fn retire(&mut self, edge: ribbit::Packed<Edge>) {
        if edge.meta().kind() < node::Kind::NODE_3 {
            return;
        }

        stat::increment(stat::Counter::Retire);

        self.0.edges.push(edge);

        if self.0.edges.len() >= RETIRED_COUNT {
            self.flush();
        }
    }

    #[cold]
    fn flush(&mut self) {
        stat::max(stat::Max::RetireCache, self.0.edges.len() as u64);
        stat::increment(stat::Counter::Flush);

        membarrier::slow();

        let hazards = self
            .0
            .hazards
            .iter()
            .map(|hazard| hazard.0.load_packed(Ordering::Relaxed))
            .filter(|hazard| *hazard != byte::Array::EMPTY)
            .collect::<Vec<_>>();

        self.0.edges.retain(|edge| {
            if hazards
                .iter()
                .any(|hazard| hazard.has_prefix(edge.meta().key()))
            {
                stat::increment(stat::Counter::HazardMatch);
                return true;
            }

            unsafe { Edge::deallocate(*edge, stat::Counter::FreeRetire) };
            false
        })
    }
}
