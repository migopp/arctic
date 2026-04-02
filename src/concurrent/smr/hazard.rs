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

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::sync::atomic::AtomicBool;
use core::sync::atomic::Ordering;

use crate::concurrent::Smr;
use crate::concurrent::Value;
use crate::concurrent::smr;
use crate::raw::node;
use crate::stat;

#[repr(C, align(64))]
#[derive(Default)]
struct Cache<T>(T);

pub struct Hazard<P: ribbit::Pack<Packed: Prefix>, V: Value> {
    // FIXME: jagged/triangular array
    hazards: [Cache<ribbit::Atomic<P>>; smr::thread::MAX],
    locals: [UnsafeCell<Local<P, V>>; smr::thread::MAX],
    membarrier: AtomicBool,
    reclaim_threshold: usize,
    value: PhantomData<V>,
}

impl<P: ribbit::Pack<Packed: Prefix>, V: Value> Default for Hazard<P, V> {
    fn default() -> Self {
        Self {
            hazards: core::array::from_fn(|_| {
                Cache(ribbit::Atomic::new_packed(
                    <<P as ribbit::Pack>::Packed as Prefix>::HAZARD_NULL,
                ))
            }),
            locals: core::array::from_fn(|_| {
                UnsafeCell::new(Local {
                    snapshot: Vec::new(),
                    retired: Vec::new(),
                    _value: PhantomData,
                })
            }),
            membarrier: AtomicBool::new(false),
            reclaim_threshold: 64,
            value: PhantomData,
        }
    }
}

impl<P: ribbit::Pack<Packed: Prefix>, V: Value> Hazard<P, V> {
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

    /// Eagerly reclaim all retired allocations
    pub fn reclaim(&mut self) {
        self.locals
            .iter_mut()
            .take(smr::thread::count())
            .map(|local| local.get_mut())
            .flat_map(|local| local.retired.drain(..))
            .for_each(|(prefix, raw)| {
                deallocate::<P, V>(prefix, raw, stat::Counter::FreeReclaim);
            })
    }
}

impl<P: ribbit::Pack<Packed: Prefix>, V: Value> Drop for Hazard<P, V> {
    fn drop(&mut self) {
        self.reclaim();
    }
}

impl<P: ribbit::Pack<Packed: Prefix>, V: Value> Smr<P, V> for Hazard<P, V> {
    type Local<'g>
        = &'g Hazard<P, V>
    where
        Self: 'g;

    fn local<'g>(&'g self) -> Self::Local<'g> {
        self
    }
}

#[repr(align(64))]
pub struct Local<P: ribbit::Pack<Packed: Prefix>, V> {
    snapshot: Vec<ribbit::Packed<P>>,
    retired: Vec<(ribbit::Packed<P>, u64)>,
    _value: PhantomData<V>,
}

impl<P: ribbit::Pack<Packed: Prefix>, V: Value> Hazard<P, V> {
    #[inline]
    pub fn enable_membarrier(&self) {
        self.membarrier.store(true, Ordering::Relaxed)
    }

    #[cold]
    fn flush(&self) {
        let id = smr::thread::Id::current();
        let local = unsafe { &mut *self.locals[usize::from(id)].get() };

        stat::max(stat::Max::RetireCache, local.retired.len() as u64);

        membarrier::slow(self.membarrier.load(Ordering::Relaxed));

        local.snapshot.extend(
            self.hazards[..usize::from(id)]
                .iter()
                .chain(&self.hazards[usize::from(id) + 1..smr::thread::count()])
                .map(|hazard| hazard.0.load_packed(Ordering::Relaxed))
                .filter(|hazard| hazard.is_active()),
        );

        let mut freed = 0;

        local.retired.retain_mut(|(prefix, raw)| {
            if local
                .snapshot
                .iter()
                .any(|hazard| hazard.is_conflict(*prefix))
            {
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

            deallocate::<P, V>(*prefix, *raw, stat::Counter::FreeRetire);
            false
        });

        local.snapshot.clear();
        stat::record(stat::Record::Flush, freed);
    }
}

impl<'g, P: ribbit::Pack<Packed: Prefix>, V: Value> smr::Local<P, V> for &'g Hazard<P, V> {
    type Guard<'l>
        = Guard<'g, 'l, P, V>
    where
        Self: 'l;

    #[inline]
    fn guard<'l>(&'l mut self, hazard: ribbit::Packed<P>) -> Self::Guard<'l> {
        let id = smr::thread::Id::current();
        self.hazards[usize::from(id)]
            .0
            .store_packed(hazard, Ordering::Relaxed);
        membarrier::fast(self.membarrier.load(Ordering::Relaxed));
        Guard(self)
    }
}

pub struct Guard<'g, 'l, P: ribbit::Pack<Packed: Prefix>, V: Value>(&'l mut &'g Hazard<P, V>);

impl<'g, 'l, P: ribbit::Pack<Packed: Prefix>, V: Value> smr::Guard<V> for Guard<'g, 'l, P, V> {
    #[expect(private_bounds)]
    #[expect(private_interfaces)]
    unsafe fn retire_node<M: ribbit::Pack<Packed: crate::raw::edge::Meta>>(
        &mut self,
        _bits: usize,
        node: ribbit::Packed<node::Ptr<M>>,
    ) {
        stat::increment(stat::Counter::Retire);

        {
            let id = smr::thread::Id::current();
            let local = unsafe { &mut *self.0.locals[usize::from(id)].get() };
            let hazard = &self.0.hazards[usize::from(id)].0;

            let prefix = hazard
                .load_packed(Ordering::Relaxed)
                .into_prefix(false, Some(_bits));

            local.retired.push((prefix, node.raw().get()));

            if local.retired.len() < self.0.reclaim_threshold {
                return;
            }
        }

        self.0.flush();
    }

    unsafe fn retire_value(&mut self, value: u64) {
        stat::increment(stat::Counter::Retire);

        {
            let id = smr::thread::Id::current();
            let local = unsafe { &mut *self.0.locals[usize::from(id)].get() };
            let hazard = &self.0.hazards[usize::from(id)].0;

            let prefix = hazard
                .load_packed(Ordering::Relaxed)
                .into_prefix(true, None);

            local.retired.push((prefix, value));

            if local.retired.len() < self.0.reclaim_threshold {
                return;
            }
        }

        self.0.flush();
    }
}

impl<'g, 'l, P: ribbit::Pack<Packed: Prefix>, V: Value> Drop for Guard<'g, 'l, P, V> {
    fn drop(&mut self) {
        let id = smr::thread::Id::current();
        let hazard = &self.0.hazards[usize::from(id)].0;

        hazard.store_packed(ribbit::Packed::<P>::HAZARD_NULL, Ordering::Relaxed);
    }
}

fn deallocate<P: ribbit::Pack<Packed: Prefix>, V: Value>(
    prefix: ribbit::Packed<P>,
    raw: u64,
    counter: stat::Counter,
) {
    if prefix.is_node() {
        unsafe {
            // FIXME: type of edge meta is irrelevant here
            crate::raw::node::Ptr::<crate::raw::edge::Be>::new_unchecked(raw).deallocate(counter);
        }
    } else {
        unsafe {
            stat::increment(counter);
            drop(V::from_raw(raw));
        }
    }
}
