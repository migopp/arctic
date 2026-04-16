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

pub struct Global<P: ribbit::Pack<Packed: Prefix>, V: Value> {
    // FIXME: jagged/triangular array
    hazards: [Cache<ribbit::Atomic<P>>; smr::thread::MAX],
    locals: [UnsafeCell<Local<P, V>>; smr::thread::MAX],
    membarrier: AtomicBool,
    reclaim_threshold: usize,
    value: PhantomData<V>,
}

impl<P: ribbit::Pack<Packed: Prefix>, V: Value> Default for Global<P, V> {
    fn default() -> Self {
        Self {
            hazards: core::array::from_fn(|_| {
                Cache(ribbit::Atomic::new_packed(
                    <<P as ribbit::Pack>::Packed as Prefix>::HAZARD_NULL,
                ))
            }),
            locals: core::array::from_fn(|_| {
                UnsafeCell::new(Local {
                    cycle: 0,
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
        let hazard = &self.hazards[id].0;
        let local = &self.locals[id];

        validate!(!hazard.load_packed(Ordering::Relaxed).is_active());
        hazard.store_packed(prefix, Ordering::Relaxed);
        membarrier::fast(self.membarrier.load(Ordering::Relaxed));

        Guard {
            hazard,
            local,
            global: self,
        }
    }
}

#[repr(align(64))]
pub struct Local<P: ribbit::Pack<Packed: Prefix>, V> {
    cycle: usize,
    snapshot: Vec<ribbit::Packed<P>>,
    retired: Vec<(ribbit::Packed<P>, u64)>,
    _value: PhantomData<V>,
}

impl<P: ribbit::Pack<Packed: Prefix>, V: Value> Global<P, V> {
    #[inline]
    pub fn enable_membarrier(&self) {
        self.membarrier.store(true, Ordering::Relaxed)
    }

    #[cold]
    fn flush(global: &Global<P, V>, local: &mut Local<P, V>) {
        stat::max(stat::Max::RetireCache, local.retired.len() as u64);

        membarrier::slow(global.membarrier.load(Ordering::Relaxed));

        local.snapshot.extend(
            global.hazards[..smr::thread::count().next_multiple_of(4)]
                .iter()
                .map(|hazard| hazard.0.load_packed(Ordering::Relaxed)),
        );

        let mut freed = 0;
        let (chunks, leftover) = local.snapshot.as_chunks::<4>();
        validate!(leftover.is_empty());

        local.retired.retain_mut(|(prefix, raw)| {
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

            deallocate::<P, V>(*prefix, *raw, stat::Counter::FreeRetire);
            false
        });

        local.snapshot.clear();
        stat::record(stat::Record::Flush, freed);
    }
}

pub struct Guard<'g, P: ribbit::Pack<Packed: Prefix>, V: Value> {
    hazard: &'g ribbit::Atomic<P>,
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

        unsafe { &mut *self.local.get() }
            .retired
            .push((prefix, node.raw().get()));
    }

    unsafe fn retire_value(&mut self, value: u64) {
        stat::increment(stat::Counter::Retire);

        let prefix = self
            .hazard
            .load_packed(Ordering::Relaxed)
            .into_prefix(true, None);

        unsafe { &mut *self.local.get() }
            .retired
            .push((prefix, value));
    }
}

impl<'g, P: ribbit::Pack<Packed: Prefix>, V: Value> Drop for Guard<'g, P, V> {
    fn drop(&mut self) {
        self.hazard
            .store_packed(ribbit::Packed::<P>::HAZARD_NULL, Ordering::Relaxed);

        let local = unsafe { &mut *self.local.get() };
        if local.retired.len() < self.global.reclaim_threshold {
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
            crate::raw::node::Ptr::<crate::raw::edge::Be>::new_unchecked(raw).deallocate(counter);
        }
    } else {
        unsafe {
            stat::increment(counter);
            drop(V::from_raw(raw));
        }
    }
}
