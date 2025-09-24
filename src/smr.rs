use core::cell::RefCell;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic64;
use thread_local::ThreadLocal;

use crate::key;
use crate::membarrier;
use crate::node;
use crate::stat;
use crate::Edge;

const RETIRED_COUNT: usize = 16;

#[repr(C, align(64))]
#[derive(Default)]
struct Cache<T>(T);

pub(crate) struct State {
    hazards: ThreadLocal<Cache<Atomic64<key::Array>>>,
    retired: ThreadLocal<Cache<RefCell<Vec<ribbit::Packed<Edge>>>>>,
}

impl State {
    #[inline]
    pub(crate) fn protect<K: key::Iterator>(&self, key: &K) -> Guard {
        let prefix = key.prefix(key::Array::MAX_LEN);
        let hazard = &self.hazards.get_or_default().0;
        hazard.store_packed(prefix, Ordering::Relaxed);
        membarrier::fast();
        Guard {
            state: self,
            hazard,
        }
    }
}

impl Default for State {
    fn default() -> Self {
        Self {
            hazards: ThreadLocal::with_capacity(128),
            retired: ThreadLocal::with_capacity(128),
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        self.retired
            .iter_mut()
            .map(|Cache(retired)| retired)
            .flat_map(RefCell::get_mut)
            .for_each(|edge| unsafe { Edge::deallocate(*edge) })
    }
}

pub(crate) struct Guard<'a> {
    state: &'a State,
    hazard: &'a Atomic64<key::Array>,
}

impl Drop for Guard<'_> {
    #[inline]
    fn drop(&mut self) {
        self.hazard
            .store_packed(key::Array::EMPTY, Ordering::Relaxed);
    }
}

impl Guard<'_> {
    pub(crate) unsafe fn retire(&self, edge: ribbit::Packed<Edge>) {
        if edge.meta().kind() < node::Kind::NODE_3 {
            return;
        }

        stat::increment(stat::Counter::Retire);

        let mut retired = self.state.retired.get_or_default().0.borrow_mut();
        retired.push(edge);
        if retired.len() >= RETIRED_COUNT {
            drop(retired);
            self.flush();
        }
    }

    #[cold]
    fn flush(&self) {
        membarrier::slow();

        let hazards = self
            .state
            .hazards
            .iter()
            .map(|hazard| hazard.0.load_packed(Ordering::Relaxed))
            .filter(|hazard| *hazard != key::Array::EMPTY)
            .collect::<Vec<_>>();

        self.state
            .retired
            .get_or_default()
            .0
            .borrow_mut()
            .retain(|edge| {
                if hazards
                    .iter()
                    .any(|hazard| key::Array::has_prefix(*hazard, edge.meta().key()))
                {
                    return true;
                }

                unsafe { Edge::deallocate(*edge) };
                false
            })
    }
}
