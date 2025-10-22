use core::cell::RefCell;
use core::fmt;
use core::sync::atomic::AtomicU64;
use core::sync::atomic::Ordering;

use thread_local::ThreadLocal;

use crate::byte;
use crate::smr::membarrier;
use crate::stat;
use crate::Edge;
use crate::Value;

const RETIRED_COUNT: usize = 16;

#[repr(C, align(64))]
#[derive(Default)]
struct Cache<T>(T);

pub(crate) struct Global<V> {
    hazards: ThreadLocal<Cache<AtomicU64>>,
    edges: ThreadLocal<Cache<RefCell<Vec<ribbit::Packed<Edge<V>>>>>>,
}

impl<V> Global<V> {
    pub(crate) fn pin(&self) -> Local<V> {
        Local {
            hazards: &self.hazards,
            hazard: &self.hazards.get_or_default().0,
            edges: self.edges.get_or_default().0.borrow_mut(),
        }
    }
}

impl<V> Default for Global<V> {
    fn default() -> Self {
        Self {
            hazards: ThreadLocal::with_capacity(128),
            edges: ThreadLocal::with_capacity(128),
        }
    }
}

impl<V> Drop for Global<V> {
    fn drop(&mut self) {
        self.edges
            .iter_mut()
            .map(|Cache(retired)| retired)
            .flat_map(RefCell::get_mut)
            .for_each(|edge| unsafe { edge.data().deallocate_unchecked(stat::Counter::FreeDrop) })
    }
}

pub(crate) struct Local<'g, V: 'g> {
    hazards: &'g ThreadLocal<Cache<AtomicU64>>,
    hazard: &'g AtomicU64,
    edges: std::cell::RefMut<'g, Vec<ribbit::Packed<Edge<V>>>>,
}

impl<'g, V> Local<'g, V> {
    #[inline]
    pub(crate) fn protect<'l>(&'l mut self, prefix: byte::Array) -> PathGuard<'g, 'l, V> {
        self.hazard
            .store(prefix.value() | MASK_VALID, Ordering::Relaxed);
        membarrier::fast();
        PathGuard(Some(self))
    }

    fn unprotect(&mut self) {
        self.hazard
            .store(byte::Array::EMPTY.value(), Ordering::Relaxed);
    }

    fn retire(&mut self, edge: ribbit::Packed<Edge<V>>) {
        validate!(edge.is_node());

        stat::increment(stat::Counter::Retire);

        self.edges.push(edge);

        if self.edges.len() >= RETIRED_COUNT {
            self.flush();
        }
    }

    #[cold]
    fn flush(&mut self) {
        stat::max(stat::Max::RetireCache, self.edges.len() as u64);
        stat::increment(stat::Counter::Flush);

        membarrier::slow();

        let hazards = self
            .hazards
            .iter()
            .map(|hazard| hazard.0.load(Ordering::Relaxed))
            .filter(|hazard| hazard & MASK_VALID > 0)
            .map(byte::Array::new_masked)
            .collect::<Vec<_>>();

        self.edges.retain(|edge| {
            if hazards
                .iter()
                .any(|hazard| hazard.is_overlapping(edge.meta().key()))
            {
                stat::increment(stat::Counter::HazardMatch);
                return true;
            }

            unsafe { edge.data().deallocate_unchecked(stat::Counter::FreeRetire) };
            false
        })
    }
}

const MASK_VALID: u64 = 0b0100_0000;
const _: () = assert!(MASK_VALID & byte::Array::MASK == 0);

pub struct PathGuard<'g, 'l, V: 'g>(Option<&'l mut Local<'g, V>>);

impl<V> Drop for PathGuard<'_, '_, V> {
    #[inline]
    fn drop(&mut self) {
        if let Some(local) = &mut self.0 {
            local.unprotect();
        }
    }
}

impl<'g, 'l, V> PathGuard<'g, 'l, V> {
    pub(crate) unsafe fn retire(&mut self, edge: ribbit::Packed<Edge<V>>) {
        let local = self.0.as_mut().unwrap_unchecked();
        local.retire(edge);
    }
}

impl<'g, 'l, V: Value> PathGuard<'g, 'l, V> {
    pub(crate) unsafe fn own(mut self, value: V::Ref<'g>) -> LeafGuard<'g, 'l, true, V> {
        LeafGuard {
            local: self.0.take().unwrap_unchecked(),
            value,
        }
    }

    pub(crate) unsafe fn share(mut self, value: V::Ref<'g>) -> LeafGuard<'g, 'l, false, V> {
        LeafGuard {
            local: self.0.take().unwrap_unchecked(),
            value,
        }
    }
}

pub struct LeafGuard<'g, 'l, const RETIRE: bool, V: Value> {
    local: &'l mut Local<'g, V>,
    value: V::Ref<'g>,
}

impl<'g, const RETIRE: bool, V> fmt::Debug for LeafGuard<'g, '_, RETIRE, V>
where
    V: Value,
    V::Ref<'g>: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)
    }
}

impl<'g, 'l, const RETIRE: bool, V> LeafGuard<'g, 'l, RETIRE, V>
where
    V: Value,
{
    pub fn as_ref(&self) -> V::Ref<'g> {
        self.value
    }
}

impl<'g, 'l, const RETIRE: bool, V> core::ops::Deref for LeafGuard<'g, 'l, RETIRE, V>
where
    V: Value,
{
    type Target = V::Ref<'g>;
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<'g, 'l, const RETIRE: bool, V> Drop for LeafGuard<'g, 'l, RETIRE, V>
where
    V: Value,
{
    fn drop(&mut self) {
        self.local.unprotect();
    }
}
