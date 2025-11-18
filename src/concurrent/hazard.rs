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
//!
//! We use guard types to ensure that a hazard prefix is installed
//! for the lifetime of an operation. There are three types of guards.
//!
//! # Traversal guard
//!
//! A traversal guard is held by a cursor during traversal.
//! It protects all nodes and values with overlapping key prefixes from
//! reclamation. A traversal guard can be downgraded at runtime to
//! either a prefix guard or a value guard.
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
//! A traversal guard with key prefix `bceg` would protect
//! nodes N0 + N2 and value V2 from reclamation. A traversal
//! guard with key prefix `b` would protect nodes N0 + N2
//! and values V1 + V2 from reclamation.
//!
//! # Prefix guard
//!
//! A prefix guard is held by non-linearizable iterators like
//! [`crate::concurrent::RangeIter`]. It protects all nodes
//! and values with key prefixes underneath its key prefix from
//! reclamation.
//!
//! # Value guard
//!
//! A value guard is held by point operations and linearizable
//! guards ([`crate::concurrent::LinearizableGuard`]). It protects
//! all values with key prefixes underneath its key prefix from
//! reclamation.

pub(crate) mod guard;
mod membarrier;
pub(crate) mod prefix;

use core::marker::PhantomData;
#[cfg(not(feature = "smr-epoch"))]
use core::sync::atomic::Ordering;

use crate::concurrent;
use crate::raw::edge;
use crate::raw::Edge;
use crate::stat;

#[cfg_attr(feature = "smr-epoch", expect(dead_code))]
#[repr(C, align(64))]
#[derive(Default)]
struct Cache<T>(T);

pub(crate) struct Global<V: concurrent::Value> {
    _value: PhantomData<V>,

    #[cfg(feature = "smr-epoch")]
    collector: crossbeam_epoch::Collector,

    #[cfg(not(feature = "smr-epoch"))]
    hazards: thread_local::ThreadLocal<Cache<ribbit::Atomic<prefix::Be>>>,
    #[cfg(not(feature = "smr-epoch"))]
    retired: thread_local::ThreadLocal<
        Cache<core::cell::RefCell<Vec<(ribbit::Packed<prefix::Be>, u64)>>>,
    >,
    #[cfg(not(feature = "smr-epoch"))]
    reclaim_threshold: usize,
}

impl<V: concurrent::Value> Global<V> {
    pub(crate) fn with_reclaim_threshold(_reclaim_threshold: usize) -> Self {
        Self {
            _value: PhantomData,

            #[cfg(feature = "smr-epoch")]
            collector: crossbeam_epoch::Collector::default(),

            #[cfg(not(feature = "smr-epoch"))]
            hazards: thread_local::ThreadLocal::with_capacity(128),
            #[cfg(not(feature = "smr-epoch"))]
            retired: thread_local::ThreadLocal::with_capacity(128),
            #[cfg(not(feature = "smr-epoch"))]
            reclaim_threshold: _reclaim_threshold,
        }
    }

    pub(crate) fn pin(&self) -> Local<V> {
        Local {
            _value: PhantomData,

            #[cfg(feature = "smr-epoch")]
            handle: self.collector.register(),

            #[cfg(not(feature = "smr-epoch"))]
            hazards: &self.hazards,
            #[cfg(not(feature = "smr-epoch"))]
            hazard: &self
                .hazards
                .get_or(|| Cache(ribbit::Atomic::new_packed(prefix::Be::HAZARD_NULL)))
                .0,
            #[cfg(not(feature = "smr-epoch"))]
            retired: self.retired.get_or_default().0.borrow_mut(),
            #[cfg(not(feature = "smr-epoch"))]
            reclaim_threshold: self.reclaim_threshold,
        }
    }
}

impl<V: concurrent::Value> Default for Global<V> {
    fn default() -> Self {
        Self::with_reclaim_threshold(64)
    }
}

#[cfg(not(feature = "smr-epoch"))]
impl<V: concurrent::Value> Drop for Global<V> {
    fn drop(&mut self) {
        self.retired
            .iter_mut()
            .map(|Cache(retired)| retired)
            .flat_map(core::cell::RefCell::get_mut)
            .for_each(|(prefix, raw)| {
                unsafe { deallocate_hazard::<V>(*prefix, *raw, stat::Counter::FreeDrop) };
            })
    }
}

pub(crate) struct Local<'g, V> {
    _value: PhantomData<&'g V>,

    #[cfg(feature = "smr-epoch")]
    handle: crossbeam_epoch::LocalHandle,

    #[cfg(not(feature = "smr-epoch"))]
    hazards: &'g thread_local::ThreadLocal<Cache<ribbit::Atomic<prefix::Be>>>,
    #[cfg(not(feature = "smr-epoch"))]
    hazard: &'g ribbit::Atomic<prefix::Be>,
    #[cfg(not(feature = "smr-epoch"))]
    retired: std::cell::RefMut<'g, Vec<(ribbit::Packed<prefix::Be>, u64)>>,
    #[cfg(not(feature = "smr-epoch"))]
    reclaim_threshold: usize,
}

#[cfg(feature = "smr-epoch")]
impl<'g, V: concurrent::Value> Local<'g, V> {
    #[inline]
    pub(crate) fn guard<'l>(&'l mut self) -> guard::Traverse<'g, 'l, V> {
        guard::Traverse::new(self)
    }
}

#[cfg(not(feature = "smr-epoch"))]
impl<'g, V: concurrent::Value> Local<'g, V> {
    #[inline]
    pub(crate) fn guard<'l>(
        &'l mut self,
        prefix: ribbit::Packed<prefix::Be>,
    ) -> guard::Traverse<'g, 'l, V> {
        self.hazard.store_packed(prefix, Ordering::Relaxed);
        membarrier::fast();
        guard::Traverse::new(self)
    }

    unsafe fn retire_edge<M: ribbit::Pack<Packed: edge::Meta>>(
        &mut self,
        _bits: usize,
        edge: ribbit::Packed<Edge<M>>,
    ) {
        validate!(!edge.is_null());
        stat::increment(stat::Counter::Retire);

        {
            use crate::raw::edge::Meta as _;

            let prefix = self
                .hazard
                .load_packed(Ordering::Relaxed)
                .into_prefix(edge.meta().is_value(), Some(_bits));

            self.retired.push((prefix, edge.into_raw()));

            if self.retired.len() >= self.reclaim_threshold {
                self.flush();
            }
        }
    }

    unsafe fn retire_value(&mut self, raw: u64) {
        stat::increment(stat::Counter::Retire);

        let prefix = self
            .hazard
            .load_packed(Ordering::Relaxed)
            .into_prefix(true, None);

        self.retired.push((prefix, raw));

        if self.retired.len() >= self.reclaim_threshold {
            self.flush();
        }
    }

    #[cold]
    fn flush(&mut self) {
        stat::max(stat::Max::RetireCache, self.retired.len() as u64);

        membarrier::slow();

        let hazards = self
            .hazards
            .iter()
            .map(|hazard| hazard.0.load_packed(Ordering::Relaxed))
            .filter(|hazard| hazard.is_active())
            .collect::<Vec<_>>();

        let mut freed = 0;

        self.retired.retain(|(prefix, raw)| {
            if hazards.iter().any(|hazard| hazard.is_conflict(*prefix)) {
                stat::increment(stat::Counter::HazardMatch);
                return true;
            }

            freed += 1;
            unsafe { deallocate_hazard::<V>(*prefix, *raw, stat::Counter::FreeRetire) };
            false
        });

        stat::record(stat::Record::Flush, freed);
    }
}

#[cfg_attr(feature = "smr-epoch", expect(dead_code))]
unsafe fn deallocate_hazard<V: concurrent::Value>(
    prefix: ribbit::Packed<prefix::Be>,
    raw: u64,
    counter: stat::Counter,
) {
    validate!(prefix.value() ^ prefix.node());

    if prefix.node() {
        unsafe {
            // FIXME: type of edge meta is irrelevant here
            crate::raw::edge::Node::<crate::raw::edge::Be>::new_unchecked(raw)
                .deallocate_unchecked(counter);
        }
    } else {
        unsafe {
            stat::increment(counter);
            drop(V::from_raw(raw));
        }
    }
}

#[cfg_attr(not(feature = "smr-epoch"), expect(dead_code))]
unsafe fn deallocate_epoch<M: ribbit::Pack<Packed: edge::Meta>, V: concurrent::Value>(
    edge: ribbit::Packed<Edge<M>>,
) {
    match edge.child() {
        None => unreachable!(),
        Some(edge::Child::Value(value)) => unsafe {
            stat::increment(stat::Counter::FreeRetire);
            drop(V::from_raw(value));
        },
        Some(edge::Child::Node(node)) => unsafe {
            node.deallocate_unchecked(stat::Counter::FreeRetire);
        },
    }
}
