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
    pub(crate) fn guard<'l>(&'l mut self, prefix: byte::Array) -> TraverseGuard<'g, 'l, V> {
        self.hazard
            .store(prefix.value() | MASK_VALID, Ordering::Relaxed);
        membarrier::fast();
        TraverseGuard(self)
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

pub struct TraverseGuard<'g, 'l, V: Value>(&'l mut Local<'g, V>);

impl<V: Value> Drop for TraverseGuard<'_, '_, V> {
    #[inline]
    fn drop(&mut self) {
        self.0
            .hazard
            .store(byte::Array::EMPTY.value(), Ordering::Relaxed);
    }
}

impl<'g, 'l, V: Value> TraverseGuard<'g, 'l, V> {
    pub(crate) fn prefix(&self) -> byte::Array {
        let prefix = self.0.hazard.load(Ordering::Relaxed);
        validate!(prefix & MASK_VALID > 0);
        byte::Array::new_masked(prefix)
    }

    pub(crate) unsafe fn retire(&mut self, edge: ribbit::Packed<Edge<V>>) {
        self.0.retire(edge);
    }

    /// # SAFETY
    ///
    /// Caller must ensure that only one thread calls this for any given value.
    #[inline]
    pub(crate) unsafe fn guard_owned(self, value: V::Borrow<'l>) -> ValueGuard<'g, 'l, true, V> {
        ValueGuard { inner: self, value }
    }

    #[inline]
    pub(crate) fn guard_shared(self, value: V::Borrow<'l>) -> ValueGuard<'g, 'l, false, V> {
        ValueGuard { inner: self, value }
    }

    #[inline]
    pub(crate) fn guard_prefix(self) -> PrefixGuard<'g, 'l, V> {
        PrefixGuard(self)
    }
}

pub struct PrefixGuard<'g, 'l, V: Value>(TraverseGuard<'g, 'l, V>);

impl<'g, 'l, V: Value> core::ops::Deref for PrefixGuard<'g, 'l, V> {
    type Target = TraverseGuard<'g, 'l, V>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub struct ValueGuard<'g, 'l, const OWNED: bool, V: Value> {
    inner: TraverseGuard<'g, 'l, V>,
    value: V::Borrow<'l>,
}

impl<'l, const OWNED: bool, V> fmt::Debug for ValueGuard<'_, 'l, OWNED, V>
where
    V: Value,
    V::Borrow<'l>: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)
    }
}

impl<'g, 'l, const OWNED: bool, V> core::ops::Deref for ValueGuard<'g, 'l, OWNED, V>
where
    V: Value,
{
    type Target = V::Borrow<'l>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<'g, 'l, const OWNED: bool, V> Drop for ValueGuard<'g, 'l, OWNED, V>
where
    V: Value,
{
    fn drop(&mut self) {
        validate!(self.inner.0.hazard.load(Ordering::Relaxed) & MASK_VALID > 0);

        if OWNED {
            let key = self.inner.0.hazard.load(Ordering::Relaxed);
            validate!(key & MASK_VALID > 0);
            let key = byte::Array::new_masked(key);

            // NOTE: could technically unguard before retiring, since
            // we will not access `value` anymore, but then we'd want
            // to avoid dropping `self.inner`.
            self.inner.0.retire(ribbit::Packed::<Edge<V>>::new(
                edge::Meta::LEAF.with_key(key),
                edge::Data::from_borrow(self.value),
            ))
        }
    }
}
