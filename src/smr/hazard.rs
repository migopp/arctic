use core::cell::RefCell;
use core::sync::atomic::AtomicU64;
use core::sync::atomic::Ordering;

use thread_local::ThreadLocal;

use crate::byte;
use crate::smr::membarrier;
use crate::stat;
use crate::Edge;

const RETIRED_COUNT: usize = 16;

#[repr(C, align(64))]
#[derive(Default)]
struct Cache<T>(T);

pub(crate) struct Global {
    hazards: ThreadLocal<Cache<AtomicU64>>,
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
            .for_each(|edge| unsafe { Edge::deallocate_unchecked(*edge, stat::Counter::FreeDrop) })
    }
}

pub(crate) struct Local<'g> {
    hazards: &'g ThreadLocal<Cache<AtomicU64>>,
    hazard: &'g AtomicU64,
    edges: std::cell::RefMut<'g, Vec<ribbit::Packed<Edge>>>,
}

impl<'g> Local<'g> {
    #[inline]
    pub(crate) fn protect<'l>(&'l mut self, prefix: byte::Array) -> Guard<'g, 'l> {
        self.hazard.store(prefix.value(), Ordering::Relaxed);
        membarrier::fast();
        Guard(self)
    }
}

pub(crate) struct Guard<'g, 'l>(&'l mut Local<'g>);

impl Drop for Guard<'_, '_> {
    #[inline]
    fn drop(&mut self) {
        self.0
            .hazard
            .store(byte::Array::EMPTY.value(), Ordering::Relaxed);
    }
}

impl Guard<'_, '_> {
    pub(crate) unsafe fn retire(&mut self, edge: ribbit::Packed<Edge>) {
        if edge.meta().leaf() || edge.data() == 0 {
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
            .map(|hazard| hazard.0.load(Ordering::Relaxed))
            .map(|hazard| unsafe { byte::Array::new_unchecked(hazard) })
            .filter(|hazard| *hazard != byte::Array::EMPTY)
            .collect::<Vec<_>>();

        self.0.edges.retain(|edge| {
            if hazards
                .iter()
                .any(|hazard| hazard.is_overlapping(edge.meta().key()))
            {
                stat::increment(stat::Counter::HazardMatch);
                return true;
            }

            unsafe { Edge::deallocate_unchecked(*edge, stat::Counter::FreeRetire) };
            false
        })
    }
}
