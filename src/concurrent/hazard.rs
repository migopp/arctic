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

use core::cell::RefCell;
use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use ribbit::Atomic;
use thread_local::ThreadLocal;

use crate::concurrent;
use crate::raw::edge;
use crate::raw::edge::Meta as _;
use crate::raw::Edge;
use crate::stat;

#[repr(C, align(64))]
#[derive(Default)]
struct Cache<T>(T);

pub(crate) struct Global<V: concurrent::Value> {
    _value: PhantomData<V>,
    hazards: ThreadLocal<Cache<Atomic<prefix::Be>>>,
    retired: ThreadLocal<Cache<RefCell<Vec<(ribbit::Packed<prefix::Be>, u64)>>>>,
    reclaim_threshold: usize,
}

impl<V: concurrent::Value> Global<V> {
    pub(crate) fn with_reclaim_threshold(reclaim_threshold: usize) -> Self {
        Self {
            _value: PhantomData,
            hazards: ThreadLocal::with_capacity(128),
            retired: ThreadLocal::with_capacity(128),
            reclaim_threshold,
        }
    }

    pub(crate) fn pin(&self) -> Local<V> {
        Local {
            _value: PhantomData,
            hazards: &self.hazards,
            hazard: &self
                .hazards
                .get_or(|| Cache(Atomic::new_packed(prefix::Be::HAZARD_NULL)))
                .0,
            retired: self.retired.get_or_default().0.borrow_mut(),
            reclaim_threshold: self.reclaim_threshold,
        }
    }
}

impl<V: concurrent::Value> Default for Global<V> {
    fn default() -> Self {
        Self::with_reclaim_threshold(16)
    }
}

impl<V: concurrent::Value> Drop for Global<V> {
    fn drop(&mut self) {
        self.retired
            .iter_mut()
            .map(|Cache(retired)| retired)
            .flat_map(RefCell::get_mut)
            .for_each(|(prefix, raw)| {
                unsafe { deallocate::<V>(*prefix, *raw) };
            })
    }
}

pub(crate) struct Local<'g, V: 'g> {
    _value: PhantomData<V>,
    hazards: &'g ThreadLocal<Cache<Atomic<prefix::Be>>>,
    hazard: &'g Atomic<prefix::Be>,
    retired: std::cell::RefMut<'g, Vec<(ribbit::Packed<prefix::Be>, u64)>>,
    reclaim_threshold: usize,
}

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
        bits: usize,
        edge: ribbit::Packed<Edge<M>>,
    ) {
        validate!(!edge.is_null());
        stat::increment(stat::Counter::Retire);

        let prefix = self
            .hazard
            .load_packed(Ordering::Relaxed)
            .into_prefix(edge.meta().is_value(), Some(bits));

        self.retired.push((prefix, edge.into_raw()));

        if self.retired.len() >= self.reclaim_threshold {
            self.flush();
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
        stat::increment(stat::Counter::Flush);

        membarrier::slow();

        let hazards = self
            .hazards
            .iter()
            .map(|hazard| hazard.0.load_packed(Ordering::Relaxed))
            .filter(|hazard| hazard.is_active())
            .collect::<Vec<_>>();

        self.retired.retain(|(prefix, raw)| {
            if hazards.iter().any(|hazard| hazard.is_conflict(*prefix)) {
                stat::increment(stat::Counter::HazardMatch);
                return true;
            }

            unsafe { deallocate::<V>(*prefix, *raw) };
            false
        })
    }
}

unsafe fn deallocate<V: concurrent::Value>(prefix: ribbit::Packed<prefix::Be>, raw: u64) {
    validate!(prefix.value() ^ prefix.node());

    if prefix.node() {
        unsafe {
            // FIXME: type of edge meta is irrelevant here
            crate::raw::edge::Node::<crate::raw::edge::Be>::new_unchecked(raw)
                .deallocate_unchecked(stat::Counter::FreeRetire);
        }
    } else {
        unsafe {
            stat::increment(stat::Counter::FreeRetire);
            drop(V::from_raw(raw));
        }
    }
}
