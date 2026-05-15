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

#[cfg(feature = "opt-batch")]
mod batch;

#[cfg(feature = "opt-epoch")]
mod epoch;

mod membarrier;
pub mod prefix;
pub(crate) use prefix::Prefix;

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::sync::atomic::AtomicBool;
use core::sync::atomic::AtomicU64;
use core::sync::atomic::Ordering;

use std::collections::VecDeque;

use crate::concurrent::Smr;
use crate::concurrent::Value;
use crate::concurrent::smr;
use crate::raw::node;
use crate::stat;

#[cfg(feature = "opt-epoch")]
use core::sync::atomic::AtomicUsize;

#[cfg(feature = "opt-batch")]
use crossbeam_queue::ArrayQueue;

pub struct Hazard;

impl Smr for Hazard {
    type Global<P, V>
        = Box<Global<P, V>>
    where
        P: ribbit::Pack<Packed: Prefix>,
        V: Value;
}

#[repr(C, align(64))]
#[derive(Default)]
struct Cache<T>(T);

struct Slot<P: ribbit::Pack<Packed: Prefix>> {
    hazard: ribbit::Atomic<P>,
    #[cfg(feature = "opt-epoch")]
    epoch: AtomicUsize,
}

pub struct Global<P: ribbit::Pack<Packed: Prefix>, V: Value> {
    garbage: AtomicU64,

    // FIXME: jagged/triangular array
    slots: [Cache<Slot<P>>; smr::thread::MAX],
    #[cfg(feature = "opt-epoch")]
    global_epoch: Cache<AtomicUsize>,
    locals: [UnsafeCell<Local<P, V>>; smr::thread::MAX],
    membarrier: AtomicBool,
    reclaim_threshold: usize,
    #[cfg(feature = "opt-batch")]
    condemned: ArrayQueue<batch::Batch<P, V>>,
    value: PhantomData<V>,
}

unsafe impl<P: ribbit::Pack<Packed: Prefix>, V: Value> Send for Global<P, V> {}
unsafe impl<P: ribbit::Pack<Packed: Prefix>, V: Value> Sync for Global<P, V> {}

impl<P: ribbit::Pack<Packed: Prefix>, V: Value> Default for Global<P, V> {
    fn default() -> Self {
        Self {
            garbage: AtomicU64::new(0),
            slots: core::array::from_fn(|_| {
                Cache(Slot {
                    hazard: ribbit::Atomic::new_packed(
                        <<P as ribbit::Pack>::Packed as Prefix>::HAZARD_NULL,
                    ),
                    #[cfg(feature = "opt-epoch")]
                    epoch: AtomicUsize::new(0),
                })
            }),
            #[cfg(feature = "opt-epoch")]
            global_epoch: Cache(AtomicUsize::new(0)),
            locals: core::array::from_fn(|_| {
                UnsafeCell::new(Local {
                    garbage: 0,
                    cycle: 0,
                    #[cfg(feature = "opt-epoch")]
                    flushes: 0,
                    snapshot: Vec::new(),
                    retired: VecDeque::new(),
                    retired_count: 0,
                    _value: PhantomData,
                })
            }),
            membarrier: AtomicBool::new(false),
            reclaim_threshold: 64,
            #[cfg(feature = "opt-batch")]
            condemned: ArrayQueue::new(smr::thread::MAX),
            value: PhantomData,
        }
    }
}

impl<P: ribbit::Pack<Packed: Prefix>, V: Value> Global<P, V> {
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
        #[allow(unused_mut)]
        self.locals
            .iter_mut()
            .take(smr::thread::count())
            .map(|local| local.get_mut())
            .flat_map(|local| local.retired.drain(..))
            .for_each(|mut retired| {
                #[cfg(not(feature = "opt-epoch"))]
                {
                    deallocate::<P, V>(retired.0, retired.1, stat::Counter::FreeReclaim);
                }

                #[cfg(feature = "opt-epoch")]
                {
                    retired.deallocate();
                }
            });

        #[cfg(feature = "opt-batch")]
        {
            // Also free the global condemned allocations.
            //
            // FIXME: This is buns. Doesn't implement `drain` or `iter_mut`, though...
            while !self.condemned.is_empty() {
                let mut batch = unsafe { self.condemned.pop().unwrap_unchecked() };
                batch.deallocate();
            }
        }
    }
}

impl<P: ribbit::Pack<Packed: Prefix>, V: Value> Drop for Global<P, V> {
    fn drop(&mut self) {
        self.reclaim();
    }
}

impl<P: ribbit::Pack<Packed: Prefix>, V: Value> smr::Global<P, V> for Box<Global<P, V>> {
    type Guard<'g>
        = Guard<'g, P, V>
    where
        V: 'g,
        Self: 'g;

    #[inline]
    fn guard<'g>(&'g self, prefix: ribbit::Packed<P>) -> Self::Guard<'g>
    where
        V: 'g,
    {
        let id = usize::from(smr::thread::Id::current());
        let membarrier = self.membarrier.load(Ordering::Relaxed);
        let slot = &self.slots[id].0;
        let hazard = &slot.hazard;
        #[cfg(feature = "opt-epoch")]
        let epoch = &slot.epoch;
        let local = &self.locals[id];

        #[cfg(feature = "opt-epoch")]
        let guard_epoch = self.global_epoch.0.load(Ordering::Relaxed);
        #[cfg(feature = "opt-epoch")]
        epoch.store(guard_epoch, membarrier::fast_store_ordering(membarrier));

        validate!(!hazard.load_packed(Ordering::Relaxed).is_active());
        hazard.store_packed(prefix, membarrier::fast_store_ordering(membarrier));
        membarrier::fast_barrier(membarrier);

        Guard {
            hazard,
            #[cfg(feature = "opt-epoch")]
            epoch,
            #[cfg(feature = "opt-epoch")]
            guard_epoch,
            local,
            global: self,
        }
    }

    fn garbage(&self) -> u32 {
        self.garbage.load(Ordering::Relaxed) as u32
    }
}

#[cfg(feature = "opt-epoch")]
type Retired<P, V> = epoch::EpochBatch<P, V>;

#[cfg(not(feature = "opt-epoch"))]
type Retired<P, V> = (ribbit::Packed<P>, u64, PhantomData<V>);

#[repr(align(64))]
pub struct Local<P: ribbit::Pack<Packed: Prefix>, V: Value> {
    garbage: i32,
    cycle: usize,
    #[cfg(feature = "opt-epoch")]
    flushes: usize,
    snapshot: Vec<ribbit::Packed<P>>,
    retired: VecDeque<Retired<P, V>>,
    retired_count: usize,
    _value: PhantomData<V>,
}

impl<P: ribbit::Pack<Packed: Prefix>, V: Value> Global<P, V> {
    #[inline]
    pub fn enable_membarrier(&self) {
        self.membarrier.store(true, Ordering::Relaxed)
    }

    #[cold]
    fn flush(global: &Global<P, V>, local: &mut Local<P, V>) {
        stat::max(stat::Max::RetireCache, local.retired_count as u64);

        membarrier::slow(global.membarrier.load(Ordering::Relaxed));

        // FIXME: factor out
        #[cfg(feature = "opt-epoch")]
        let global_epoch = {
            // https://github.com/kaist-cp/crossbeam/blob/master/crossbeam-epoch/src/internal.rs#L228
            let mut global_epoch = global.global_epoch.0.load(Ordering::Relaxed);
            if local.flushes % 128 == 0 {
                let advance_epoch = global.slots[..smr::thread::count()]
                    .iter()
                    .all(|slot| slot.0.epoch.load(Ordering::Relaxed) >= global_epoch);
                if advance_epoch {
                    global_epoch = match global.global_epoch.0.compare_exchange(
                        global_epoch,
                        global_epoch + 1,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    ) {
                        Ok(_) => global_epoch + 1,
                        Err(e) => e,
                    };
                }
            }
            global_epoch
        };

        local.snapshot.extend(
            global.slots[..smr::thread::count().next_multiple_of(4)]
                .iter()
                .map(|slot| slot.0.hazard.load_packed(Ordering::Relaxed)),
        );

        let mut freed = 0;
        let (chunks, leftover) = local.snapshot.as_chunks::<4>();
        validate!(leftover.is_empty());

        #[cfg(feature = "opt-epoch")]
        {
            // We get a nice shortcut here by checking the epoch of the frontmost elements in the
            // `retired` list.
            //
            // If the epoch is sufficiently old, we can skip any prefix checking and just add it to
            // the global batch list right here, right now.
            while local
                .retired
                .front_mut()
                .is_some_and(|batch| global_epoch >= batch.epoch + 2)
            {
                // Safety: Verified by above check to be present.
                let mut batch = unsafe { local.retired.pop_front().unwrap_unchecked() };
                batch.batch.iter().for_each(|(prefix, _)| {
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
                });

                // Since this epoch likely contains many retired nodes/values, we split them up
                // into batches as to not deallocate too much at once and cause strange
                // allocator behavior.
                for batch in batch.into_batches(global.reclaim_threshold) {
                    if let Err(mut batch) = global.condemned.push(batch) {
                        // No space in condemned queue. Must deallocate now.
                        batch.deallocate();
                    }
                }
            }
        }

        #[cfg(feature = "opt-batch")]
        let mut batch = Vec::new();

        local.retired.retain_mut(|retired| {
            #[cfg(not(feature = "opt-epoch"))]
            {
                let prefix = &mut retired.0;
                let raw = &mut retired.1;

                if chunks.iter().any(|chunk| prefix.is_conflict(chunk)) {
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

                #[cfg(feature = "opt-batch")]
                {
                    batch.push((*prefix, *raw));
                }

                #[cfg(not(feature = "opt-batch"))]
                {
                    deallocate::<P, V>(*prefix, *raw, stat::Counter::FreeRetire);
                }
                false
            }

            #[cfg(feature = "opt-epoch")]
            {
                retired.batch.retain_mut(|retired| {
                    let prefix = &mut retired.0;
                    let raw = &mut retired.1;

                    if chunks.iter().any(|chunk| prefix.is_conflict(chunk)) {
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

                    batch.push((*prefix, *raw));
                    false
                });
                !retired.batch.is_empty()
            }
        });

        if cfg!(feature = "stat-garbage") {
            local.garbage -= freed;

            if local.garbage <= -(global.reclaim_threshold as i32) {
                global
                    .garbage
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |garbage| {
                        let old_count = garbage >> 32;
                        let old_max = garbage as u32;

                        let new_count = old_count - ((-local.garbage) as u64);
                        Some(new_count << 32 | (old_max as u64))
                    })
                    .unwrap();
                local.garbage = 0;
            }
        }

        #[cfg(feature = "opt-batch")]
        {
            // First, push batch to the condemned queue.
            if !batch.is_empty() {
                let batch = batch::Batch::new(batch);
                if let Err(mut batch) = global.condemned.push(batch) {
                    // No space in condemned queue. Must deallocate now.
                    batch.deallocate();
                }
            }

            // Deallocate a batch, if we can.
            //
            // Goal here is to limit parallel frees.
            if !global.condemned.is_empty() {
                for _ in 0..8 {
                    if let Some(mut batch) = global.condemned.pop() {
                        batch.deallocate();
                    }
                }
            }
        }

        local.snapshot.clear();
        local.retired_count -= freed as usize;
        stat::record(stat::Record::Flush, freed as u64);
    }
}

pub struct Guard<'g, P: ribbit::Pack<Packed: Prefix>, V: Value> {
    hazard: &'g ribbit::Atomic<P>,
    #[cfg(feature = "opt-epoch")]
    epoch: &'g AtomicUsize,
    #[cfg(feature = "opt-epoch")]
    guard_epoch: usize,
    local: &'g UnsafeCell<Local<P, V>>,
    global: &'g Global<P, V>,
}

impl<'g, P: ribbit::Pack<Packed: Prefix>, V: Value> smr::Guard<V> for Guard<'g, P, V> {
    #[expect(private_bounds)]
    #[expect(private_interfaces)]
    unsafe fn retire_node<M: ribbit::Pack<Packed: crate::raw::edge::Meta>>(
        &mut self,
        _bits: usize,
        node: ribbit::Packed<node::Ptr<M>>,
    ) {
        stat::increment(stat::Counter::Retire);

        let prefix = self
            .hazard
            .load_packed(Ordering::Relaxed)
            .into_prefix(false, Some(_bits));

        let local = unsafe { &mut *self.local.get() };
        local.retired_count += 1;

        if cfg!(feature = "stat-garbage") {
            local.garbage += 1;

            if local.garbage >= self.global.reclaim_threshold as i32 {
                self.global
                    .garbage
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |garbage| {
                        let old_count = garbage >> 32;
                        let old_max = garbage as u32;

                        let new_count = old_count + local.garbage as u64;
                        let new_max = old_max.max(new_count as u32);
                        Some(new_count << 32 | (new_max as u64))
                    })
                    .unwrap();
                local.garbage = 0;
            }
        }

        #[cfg(not(feature = "opt-epoch"))]
        local
            .retired
            .push_back((prefix, node.raw().get(), PhantomData));

        #[cfg(feature = "opt-epoch")]
        {
            if let Some(batch) = local.retired.back_mut() {
                debug_assert!(batch.epoch <= self.guard_epoch);
                if batch.epoch == self.guard_epoch {
                    batch.push((prefix, node.raw().get()));
                    return;
                }
            }

            let mut batch = epoch::EpochBatch::new(self.guard_epoch, self.global.reclaim_threshold);
            batch.push((prefix, node.raw().get()));
            local.retired.push_back(batch);
        }
    }

    unsafe fn retire_value(&mut self, value: u64) {
        stat::increment(stat::Counter::Retire);

        let prefix = self
            .hazard
            .load_packed(Ordering::Relaxed)
            .into_prefix(true, None);

        let local = unsafe { &mut *self.local.get() };
        local.retired_count += 1;

        if cfg!(feature = "stat-garbage") {
            local.garbage += 1;

            if local.garbage >= self.global.reclaim_threshold as i32 {
                self.global
                    .garbage
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |garbage| {
                        let old_count = garbage >> 32;
                        let old_max = garbage as u32;

                        let new_count = old_count + local.garbage as u64;
                        let new_max = old_max.max(new_count as u32);
                        Some(new_count << 32 | (new_max as u64))
                    })
                    .unwrap();
                local.garbage = 0;
            }
        }

        #[cfg(not(feature = "opt-epoch"))]
        local.retired.push_back((prefix, value, PhantomData));

        #[cfg(feature = "opt-epoch")]
        {
            if let Some(batch) = local.retired.back_mut() {
                debug_assert!(batch.epoch <= self.guard_epoch);
                if batch.epoch == self.guard_epoch {
                    batch.push((prefix, value));
                    return;
                }
            }

            let mut batch = epoch::EpochBatch::new(self.guard_epoch, self.global.reclaim_threshold);
            batch.push((prefix, value));
            local.retired.push_back(batch);
        }
    }
}

impl<'g, P: ribbit::Pack<Packed: Prefix>, V: Value> Drop for Guard<'g, P, V> {
    fn drop(&mut self) {
        self.hazard
            .store_packed(ribbit::Packed::<P>::HAZARD_NULL, Ordering::Relaxed);

        // FIXME: clean up inactive epoch detection mechanism
        #[cfg(feature = "opt-epoch")]
        self.epoch.store(usize::MAX, Ordering::Relaxed);

        let local = unsafe { &mut *self.local.get() };
        if local.retired_count < self.global.reclaim_threshold {
            local.cycle = 0;
            return;
        }

        if local.cycle == 0 {
            Global::flush(self.global, local)
        }

        // FIXME: introduce separate configuration
        local.cycle = if local.cycle == self.global.reclaim_threshold {
            0
        } else {
            local.cycle + 1
        };
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
            crate::raw::node::Ptr::<crate::raw::edge::Be>::from_raw_unchecked(raw)
                .deallocate(counter);
        }
    } else {
        unsafe {
            stat::increment(counter);
            drop(V::from_raw(raw));
        }
    }
}
