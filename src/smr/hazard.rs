use core::cell::RefCell;
use core::fmt;
use core::sync::atomic::AtomicU64;
use core::sync::atomic::Ordering;

use thread_local::ThreadLocal;

use crate::byte;
use crate::edge;
use crate::smr::membarrier;
use crate::stat;
use crate::Edge;
use crate::Value;

const RETIRED_COUNT: usize = 16;

#[repr(C, align(64))]
#[derive(Default)]
struct Cache<T>(T);

pub(crate) struct Global<V: Value> {
    hazards: ThreadLocal<Cache<AtomicU64>>,
    edges: ThreadLocal<Cache<RefCell<Vec<ribbit::Packed<Edge<V>>>>>>,
}

impl<V: Value> Global<V> {
    pub(crate) fn pin(&self) -> Local<V> {
        Local {
            hazards: &self.hazards,
            hazard: &self.hazards.get_or_default().0,
            edges: self.edges.get_or_default().0.borrow_mut(),
        }
    }
}

impl<V: Value> Default for Global<V> {
    fn default() -> Self {
        Self {
            hazards: ThreadLocal::with_capacity(128),
            edges: ThreadLocal::with_capacity(128),
        }
    }
}

impl<V: Value> Drop for Global<V> {
    fn drop(&mut self) {
        self.edges
            .iter_mut()
            .map(|Cache(retired)| retired)
            .flat_map(RefCell::get_mut)
            .for_each(|edge| unsafe { edge.deallocate_unchecked(stat::Counter::FreeDrop) })
    }
}

pub(crate) struct Local<'g, V: 'g> {
    hazards: &'g ThreadLocal<Cache<AtomicU64>>,
    hazard: &'g AtomicU64,
    edges: std::cell::RefMut<'g, Vec<ribbit::Packed<Edge<V>>>>,
}

impl<'g, V: Value> Local<'g, V> {
    #[inline]
    pub(crate) fn protect<'l>(&'l mut self, prefix: byte::Array) -> PathGuard<'g, 'l, V> {
        self.hazard
            .store(prefix.value() | MASK_VALID, Ordering::Relaxed);
        membarrier::fast();
        PathGuard(self)
    }

    fn unprotect(&mut self) {
        self.hazard
            .store(byte::Array::EMPTY.value(), Ordering::Relaxed);
    }

    fn retire(&mut self, edge: ribbit::Packed<Edge<V>>) {
        validate!(!edge.is_null());

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

            unsafe { edge.deallocate_unchecked(stat::Counter::FreeRetire) };
            false
        })
    }
}

const MASK_VALID: u64 = 0b0100_0000;
const _: () = assert!(MASK_VALID & byte::Array::MASK == 0);

pub struct PathGuard<'g, 'l, V: Value>(&'l mut Local<'g, V>);

impl<V: Value> Drop for PathGuard<'_, '_, V> {
    #[inline]
    fn drop(&mut self) {
        self.0.unprotect();
    }
}

impl<'g, 'l, V: Value> PathGuard<'g, 'l, V> {
    pub(crate) unsafe fn retire(&mut self, edge: ribbit::Packed<Edge<V>>) {
        self.0.retire(edge);
    }
}

impl<'g, 'l, V: Value> PathGuard<'g, 'l, V> {
    #[inline]
    pub(crate) unsafe fn scope<const RETIRE: bool>(
        self,
        value: V::Borrow<'l>,
    ) -> LeafGuard<'g, 'l, RETIRE, V> {
        LeafGuard { inner: self, value }
    }
}

pub struct LeafGuard<'g, 'l, const RETIRE: bool, V: Value> {
    inner: PathGuard<'g, 'l, V>,
    value: V::Borrow<'l>,
}

impl<'l, const RETIRE: bool, V> fmt::Debug for LeafGuard<'_, 'l, RETIRE, V>
where
    V: Value,
    V::Borrow<'l>: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)
    }
}

impl<'g, 'l, const RETIRE: bool, V> LeafGuard<'g, 'l, RETIRE, V>
where
    V: Value,
{
    #[inline]
    pub fn as_ref(&self) -> V::Borrow<'l> {
        self.value
    }
}

impl<'g, 'l, const RETIRE: bool, V> core::ops::Deref for LeafGuard<'g, 'l, RETIRE, V>
where
    V: Value,
{
    type Target = V::Borrow<'l>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<'g, 'l, const RETIRE: bool, V> Drop for LeafGuard<'g, 'l, RETIRE, V>
where
    V: Value,
{
    fn drop(&mut self) {
        validate!(self.inner.0.hazard.load(Ordering::Relaxed) & MASK_VALID > 0);

        if RETIRE {
            let key = self.inner.0.hazard.load(Ordering::Relaxed);
            validate!(key & MASK_VALID > 0);
            let key = byte::Array::new_masked(key);

            // NOTE: could technically unprotect before retiring, since
            // we cannot access `value` anymore, but then we'd want
            // to avoid dropping `self.inner`.
            self.inner.0.retire(ribbit::Packed::<Edge<V>>::new(
                edge::Meta::LEAF.with_key(key),
                edge::Data::from_borrow(self.value),
            ))
        }
    }
}
