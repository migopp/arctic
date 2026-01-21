//! Unlike traditional hazard pointers, we use hazard *prefixes*,
//! which over-approxmiate a set of hazard pointers using a key prefix.
//!
//! First, note that every node and value in a trie can be associated
//! with a key prefix. For example, given the following trie:
//!
//! ```text
//!     N0 [ a | b ]
//!        /    |
//!       /     | c
//!      /      |
//!  N1 [f]  N2 [ d | e ]
//!     /        /   |
//!    /        /    | g
//!   /        /     |
//! (V0)     (V1)   (V2)
//! ```
//!
//! We have the following key prefixes:
//!
//! | Id | Type  | Prefix |
//! +----+-------|-------+
//! | N0 | Node  |       |
//! | N1 | Node  | a     |
//! | N2 | Node  | bc    |
//! | V0 | Value | af    |
//! | V1 | Value | bcd   |
//! | V2 | Value | bceg  |
//!
//! Second, note that each trie operation is also associated with
//! a key prefix. This can be a full key for point operations like
//! [`crate::concurrent::MapRef::get`], or a key prefix for prefix
//! operations like [`crate::concurrent::MapRef::prefix`].
//!
//! Then the core insight is that a trie operation will never access
//! nodes or values whose key prefixes do not overlap with its own.
//! We use guards to ensure that a hazard prefix is installed
//! for the lifetime of an operation.
//! Guards protect all nodes and values with overlapping key prefixes from
//! reclamation.
//!
//! In our example trie...
//!
//! ```text
//!     N0 [ a | b ]
//!        /    |
//!       /     | c
//!      /      |
//!  N1 [f]  N2 [ d | e ]
//!     /        /   |
//!    /        /    | g
//!   /        /     |
//! (V0)     (V1)   (V2)
//! ```
//!
//! A guard with key prefix `bceg` would protect
//! nodes N0 + N2 and value V2 from reclamation.
//! A guard with key prefix `b` would protect nodes N0 + N2
//! and values V1 + V2 from reclamation.

mod membarrier;
pub(crate) mod prefix;
pub(crate) use prefix::Prefix;

use core::marker::PhantomData;
use core::sync::atomic::AtomicBool;
use core::sync::atomic::Ordering;

use thread_local::ThreadLocal;

use crate::concurrent::smr;
use crate::concurrent::Smr;
use crate::concurrent::Value;
use crate::raw::node;
use crate::stat;

#[repr(C, align(64))]
#[derive(Default)]
struct Cache<T>(T);

pub struct Hazard<'v, P: ribbit::Pack<Packed: Prefix>, V> {
    hazards: ThreadLocal<Cache<ribbit::Atomic<P>>>,
    retired: ThreadLocal<Cache<core::cell::RefCell<Vec<(ribbit::Packed<P>, u64)>>>>,
    membarrier: AtomicBool,
    reclaim_threshold: usize,
    value: PhantomData<&'v V>,
}

impl<'v, P: ribbit::Pack<Packed: Prefix>, V> Default for Hazard<'v, P, V> {
    fn default() -> Self {
        Self {
            hazards: thread_local::ThreadLocal::with_capacity(16),
            retired: thread_local::ThreadLocal::with_capacity(16),
            membarrier: AtomicBool::new(false),
            reclaim_threshold: 64,
            value: PhantomData,
        }
    }
}

impl<'v, P: ribbit::Pack<Packed: Prefix>, V> Hazard<'v, P, V> {
    #[inline]
    #[must_use]
    pub fn with_reclaim_threshold(mut self, reclaim_threshold: usize) -> Self {
        self.reclaim_threshold = reclaim_threshold;
        self
    }

    #[inline]
    pub fn set_membarrier(&mut self, enable: bool) {
        *self.membarrier.get_mut() = enable
    }
}

impl<'v, P: ribbit::Pack<Packed: Prefix>, V: Value<'v>> Smr<'v, P, V> for Hazard<'v, P, V> {
    type Local<'g>
        = Local<'v, 'g, P, V>
    where
        Self: 'g;

    fn local<'g>(&'g self) -> Self::Local<'g> {
        Local {
            hazards: &self.hazards,
            hazard: &self
                .hazards
                .get_or(|| Cache(ribbit::Atomic::new_packed(ribbit::Packed::<P>::HAZARD_NULL)))
                .0,
            retired: self.retired.get_or_default().0.borrow_mut(),
            membarrier: &self.membarrier,
            reclaim_threshold: self.reclaim_threshold,
            value: PhantomData,
        }
    }
}

pub struct Local<'v, 'g, P: ribbit::Pack<Packed: Prefix>, V> {
    hazards: &'g thread_local::ThreadLocal<Cache<ribbit::Atomic<P>>>,
    hazard: &'g ribbit::Atomic<P>,
    retired: std::cell::RefMut<'g, Vec<(ribbit::Packed<P>, u64)>>,
    membarrier: &'g AtomicBool,
    reclaim_threshold: usize,
    value: PhantomData<&'v V>,
}

impl<'v, 'g, P: ribbit::Pack<Packed: Prefix>, V: Value<'v>> Local<'v, 'g, P, V> {
    #[inline]
    pub fn enable_membarrier(&self) {
        self.membarrier.store(true, Ordering::Relaxed)
    }

    #[cold]
    fn flush(&mut self) {
        stat::max(stat::Max::RetireCache, self.retired.len() as u64);

        membarrier::slow(self.membarrier.load(Ordering::Relaxed));

        let hazards = self
            .hazards
            .iter()
            .filter(|Cache(hazard)| !core::ptr::addr_eq(hazard, self.hazard))
            .map(|hazard| hazard.0.load_packed(Ordering::Relaxed))
            .filter(|hazard| hazard.is_active())
            .collect::<Vec<_>>();

        let mut freed = 0;

        self.retired.retain_mut(|(prefix, raw)| {
            if hazards.iter().any(|hazard| hazard.is_conflict(*prefix)) {
                stat::increment(stat::Counter::HazardMatch);
                if cfg!(feature = "stat") {
                    *prefix = prefix.with_age(prefix.age().saturating_add(1));
                }
                return true;
            }

            if cfg!(feature = "stat") {
                stat::record(stat::Record::ReclaimDepth, prefix.bytes() as u64);
            }
            freed += 1;

            validate!(prefix.is_value() ^ prefix.is_node());

            if cfg!(feature = "stat") {
                if let Some(record) = match prefix.bytes() {
                    0 => Some(stat::Record::ReclaimAge0),
                    1 => Some(stat::Record::ReclaimAge1),
                    2 => Some(stat::Record::ReclaimAge2),
                    3 => Some(stat::Record::ReclaimAge3),
                    _ => None,
                } {
                    stat::record(record, prefix.age() as u64 + 1);
                }
            }

            if prefix.is_node() {
                unsafe {
                    // FIXME: type of edge meta is irrelevant here
                    crate::raw::node::Ptr::<crate::raw::edge::Be>::new_unchecked(*raw)
                        .deallocate(stat::Counter::FreeRetire);
                }
            } else {
                unsafe {
                    stat::increment(stat::Counter::FreeRetire);
                    drop(V::from_raw(*raw));
                }
            }

            false
        });

        stat::record(stat::Record::Flush, freed);
    }
}

impl<'v, 'g, P: ribbit::Pack<Packed: Prefix>, V: Value<'v>> smr::Local<'v, P, V>
    for Local<'v, 'g, P, V>
{
    type Guard<'l>
        = Guard<'v, 'g, 'l, P, V>
    where
        Self: 'l;

    #[inline]
    fn guard<'l>(&'l mut self, hazard: ribbit::Packed<P>) -> Self::Guard<'l> {
        self.hazard.store_packed(hazard, Ordering::Relaxed);
        membarrier::fast(self.membarrier.load(Ordering::Relaxed));
        Guard(self)
    }
}

pub struct Guard<'v, 'g, 'l, P: ribbit::Pack<Packed: Prefix>, V: Value<'v>>(
    &'l mut Local<'v, 'g, P, V>,
);

impl<'v, 'g, 'l, P: ribbit::Pack<Packed: Prefix>, V: Value<'v>> smr::Guard<'v, V>
    for Guard<'v, 'g, 'l, P, V>
{
    #[expect(private_bounds)]
    #[expect(private_interfaces)]
    unsafe fn retire_node<M: ribbit::Pack<Packed: crate::raw::edge::Meta>>(
        &mut self,
        _bits: usize,
        node: ribbit::Packed<node::Ptr<M>>,
    ) {
        let prefix = self
            .0
            .hazard
            .load_packed(Ordering::Relaxed)
            .into_prefix(false, Some(_bits));

        self.0.retired.push((prefix, node.raw().get()));

        if self.0.retired.len() >= self.0.reclaim_threshold {
            self.0.flush();
        }
    }

    unsafe fn retire_value(&mut self, value: V::Borrow<'v>) {
        let prefix = self
            .0
            .hazard
            .load_packed(Ordering::Relaxed)
            .into_prefix(true, None);

        self.0.retired.push((prefix, V::borrow_into_raw(value)));

        if self.0.retired.len() >= self.0.reclaim_threshold {
            self.0.flush();
        }
    }
}

impl<'v, 'g, 'l, P: ribbit::Pack<Packed: Prefix>, V: Value<'v>> Drop for Guard<'v, 'g, 'l, P, V> {
    fn drop(&mut self) {
        self.0
            .hazard
            .store_packed(ribbit::Packed::<P>::HAZARD_NULL, Ordering::Relaxed);
    }
}
