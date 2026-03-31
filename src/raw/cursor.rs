pub(crate) mod path;

use core::marker::PhantomData;
use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use ribbit::Atomic;

use crate::raw::edge;
use crate::raw::edge::Key as _;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key::Read as _;
use crate::raw::node;
use crate::raw::node::Node3;
use crate::raw::Edge;
use crate::raw::Frozen;
use crate::raw::Key;
use crate::raw::Smo;
use crate::stat;

pub(crate) struct CursorMut<'k, 'g, K: Key>(Cursor<'k, 'g, K, path::Discard>);

impl<'k, 'g, K: Key> CursorMut<'k, 'g, K> {
    #[inline]
    pub(crate) fn new(root: &'g mut Atomic<Edge<K::Edge>>, key: K::Read<'k>) -> Self {
        Self(unsafe { Cursor::new(root, key) })
    }

    #[inline]
    pub(crate) fn edge_mut(&mut self) -> &'g mut Atomic<Edge<K::Edge>> {
        unsafe { self.0.edge.as_mut() }
    }
}

impl<'k, 'g, K: Key> core::ops::Deref for CursorMut<'k, 'g, K> {
    type Target = Cursor<'k, 'g, K, path::Discard>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'k, 'g, K: Key> core::ops::DerefMut for CursorMut<'k, 'g, K> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Tree traversal state.
pub(crate) struct Cursor<'k, 'g, K: Key, H> {
    /// Total number of bits read from `key`
    bits: usize,

    /// Current key reader
    key: K::Read<'k>,

    /// Edge this cursor currently points to
    edge: NonNull<Atomic<Edge<K::Edge>>>,

    /// Path history of this cursor (sequence of path segments)
    history: H,

    _global: PhantomData<&'g Atomic<Edge<K::Edge>>>,
}

pub(crate) enum Insert<E: ribbit::Pack<Packed: edge::Meta>> {
    Value {
        old_value: Option<u64>,
        old: ribbit::Packed<Edge<E>>,
        key: <E::Packed as edge::Meta>::Key,
    },

    /// Structural modification required
    Smo(Result<(Smo, ribbit::Packed<Edge<E>>, ribbit::Packed<Edge<E>>), Frozen>),
}

impl<'k, 'g, K, H> Cursor<'k, 'g, K, H>
where
    K: Key,
    H: path::History<'k, K>,
{
    /// # Safety
    ///
    /// Caller must ensure that all nodes underneath `root` along the path associated
    /// with `key` live at least as long as this struct.
    #[inline]
    pub(crate) unsafe fn new(root: &'g Atomic<Edge<K::Edge>>, key: K::Read<'k>) -> Self {
        Self {
            bits: 0,
            edge: NonNull::from(root),
            key,
            history: H::default(),
            _global: PhantomData,
        }
    }

    #[inline]
    pub(crate) fn edge(&self) -> &'g Atomic<Edge<K::Edge>> {
        unsafe { self.edge.as_ref() }
    }

    #[inline]
    pub(crate) fn bits(&self) -> usize {
        self.bits
    }

    /// Traverse to the value associated with the key, if it exists.
    #[inline]
    pub(crate) fn traverse_get(mut self) -> Option<u64> {
        loop {
            let edge = unsafe { self.edge.as_ref() }.load_packed(Ordering::Acquire);

            let _ = self.key.match_exact(edge.meta())?;

            match edge.child()? {
                edge::Child::Node(node) => {
                    let byte = if cfg!(feature = "validate") {
                        self.key
                            .next()
                            .expect("Precondition: no key is prefix of another key")
                    } else {
                        unsafe { self.key.next_unchecked() }
                    };

                    self.edge = unsafe { node.get(byte) }.map(NonNull::from)?;
                }
                edge::Child::Value(value) => {
                    return Some(value);
                }
            }
        }
    }

    /// Traverse to the root of the subtree prefixed by the key, if it exists.
    pub(crate) fn traverse_prefix(&mut self) -> Option<ribbit::Packed<Edge<K::Edge>>> {
        loop {
            let edge = unsafe { self.edge.as_ref() }.load_packed(Ordering::Acquire);
            let meta = edge.meta();
            let save = self.key;

            let (key, exact) = self.key.match_inexact(meta);

            if exact {
                if let Some(node) = edge.as_node() {
                    if let Some(byte) = self.key.next() {
                        let next = unsafe { node.get(byte) }?;
                        self.push(save, key.len(), node, next);
                        continue;
                    }
                }
            }

            if edge.is_null() || meta.key().prefix(key.len()) != key {
                return None;
            } else {
                self.key = save;
                return Some(edge);
            }
        }
    }

    /// Traverse to the edge associated with the key.
    ///
    /// Returns `None` if there is no such edge,
    /// `Some(Err(Frozen))` if this edge is frozen,
    /// or `Some(Ok(edge))` otherwise.
    #[inline]
    pub(crate) fn traverse_update(
        &mut self,
    ) -> Option<Result<ribbit::Packed<Edge<K::Edge>>, Frozen>> {
        loop {
            let edge = unsafe { self.edge.as_ref() }.load_packed(Ordering::Acquire);
            let meta = edge.meta();
            let save = self.key;

            // Fast path: traversal
            let len = self.key.match_exact(meta)?;

            if let Some(node) = edge.as_node() {
                let byte = if cfg!(feature = "validate") {
                    self.key
                        .next()
                        .expect("Precondition: no key is prefix of another key")
                } else {
                    unsafe { self.key.next_unchecked() }
                };

                let next = unsafe { node.get(byte) }?;
                self.push(save, len, node, next);
                continue;
            }

            self.key = save;

            return if meta.is_frozen() {
                Some(Err(Frozen))
            } else if meta.is_value() {
                Some(Ok(edge))
            } else {
                validate!(edge.is_null());
                None
            };
        }
    }

    /// Traverse to the edge associated with the key, or to
    /// the first edge where an SMO would be necessary to
    /// insert the key.
    ///
    /// NOTE: does not check for frozen if returning `Insert::Value`,
    /// since the caller may only need read access to the key.
    /// If the caller does need write access, they must check for
    /// the frozen bit before CASing.
    #[inline]
    pub(crate) fn traverse_insert(&mut self) -> Insert<K::Edge> {
        loop {
            let old = unsafe { self.edge.as_ref() }.load_packed(Ordering::Acquire);
            let old_meta = old.meta();
            let save = self.key;

            let (key, exact) = self.key.match_inexact(old_meta);

            if exact {
                if let Some(node) = old.as_node() {
                    let byte = if cfg!(feature = "validate") {
                        self.key
                            .next()
                            .expect("Precondition: no key is prefix of another key")
                    } else {
                        unsafe { self.key.next_unchecked() }
                    };

                    if let Some(next) = unsafe { node.get_or_insert(byte) } {
                        self.push(save, key.len(), node, next);
                        continue;
                    }
                }
            }

            // Revert key to before the current edge
            self.key = save;

            let old_value = match (exact, old.child()) {
                // Edge expansion or node creation
                (false, _) | (true, None) => None,
                (true, Some(edge::Child::Value(value))) => Some(value),

                // Node replacement
                (true, Some(edge::Child::Node(_))) if old_meta.is_frozen() => {
                    return Insert::Smo(Err(Frozen))
                }
                (true, Some(edge::Child::Node(node))) => {
                    let (smo, new) = unsafe { node.replace(old_meta) };
                    return Insert::Smo(Ok((smo, old, new)));
                }
            };

            return Insert::Value {
                old_value,
                old,
                key,
            };
        }
    }

    /// Locally create an edge from the current edge
    /// to the full key. May create nodes recursively if
    /// the remaining key is long.
    pub(crate) fn insert(
        &mut self,
        old: ribbit::Packed<Edge<K::Edge>>,
        key: <<K::Edge as ribbit::Pack>::Packed as edge::Meta>::Key,
        value: u64,
    ) -> Result<ribbit::Packed<Edge<K::Edge>>, Frozen> {
        if old.meta().is_frozen() {
            return Err(Frozen);
        }

        let mut save = self.key;

        let new = match old.meta().expand(key) {
            Err(_) => Edge::new_path(save, value),
            Ok((start, middle, end)) => {
                let _ = save.read(start.len());
                let byte = unsafe { save.next_unchecked() };
                Edge::new_node::<Node3<K::Edge>, _, _>(
                    start,
                    [byte, middle],
                    [Edge::new_path(save, value), old.with_meta(end)],
                )
            }
        };

        Ok(new)
    }

    /// Freeze and replace the node containing `self.edge`.
    ///
    /// Returns `Err(_)` if the path history `H` does not support popping,
    /// `Ok(Some(node))` if this thread successfully replaced `node`,
    /// or `Ok(None)` if this thread did not replace the node (e.g.,
    /// another thread won the CAS race or an edge expansion SMO pushed
    /// down the frozen node).
    #[cold]
    pub(crate) fn freeze(
        &mut self,
    ) -> Result<Option<ribbit::Packed<node::Ptr<K::Edge>>>, H::PopError> {
        let mut node = self.pop()?.expect("Root edge cannot be frozen");
        let mut edge = unsafe { self.edge.as_ref() }.load_packed(Ordering::Acquire);
        let mut pop = 1;

        let old = loop {
            while edge.meta().is_frozen() {
                node = self.pop()?.expect("Root edge cannot be frozen");
                edge = unsafe { self.edge.as_ref() }.load_packed(Ordering::Acquire);
                pop += 1;
            }

            let meta = edge.meta();

            let old = match edge.child() {
                Some(edge::Child::Node(old)) if old == node => old,
                // Child has changed since we last traversed
                // Optimistically assume that node replacement was completed by a different thread
                None | Some(_) => break None,
            };

            let (op, new) = unsafe { node.replace(meta) };

            match unsafe { self.edge.as_ref() }.compare_exchange_packed(
                edge,
                new,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    break Some(old);
                }
                Err(conflict) => {
                    if op.is_allocate() {
                        let node = new.as_node().expect("Allocating SMO creates node");
                        unsafe {
                            node.deallocate(stat::Counter::FreeConflict);
                        }
                    }
                    edge = conflict;
                }
            };
        };

        stat::record(stat::Record::FreezePop, pop);
        Ok(old)
    }

    #[inline]
    pub(crate) fn reclaim(mut self) -> Result<(), H::PopError> {
        let Some(node) = self.pop()? else {
            return Ok(());
        };

        if unsafe { node.len() } > 0 {
            return Ok(());
        }

        self.reclaim_cold(node)
    }

    #[cold]
    fn reclaim_cold(
        &mut self,
        mut node: ribbit::Packed<node::Ptr<K::Edge>>,
    ) -> Result<(), H::PopError> {
        loop {
            let old = unsafe { self.edge.as_ref() }.load_packed(Ordering::Acquire);

            let (_smo, new) = unsafe { node.replace(old.meta()) };

            match unsafe { self.edge.as_ref() }.compare_exchange_packed(
                old,
                new,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => (),
                Err(_) => {
                    if let Some(node) = new.as_node() {
                        unsafe { node.deallocate(stat::Counter::FreeConflict) };
                    }
                    self.traverse_prefix();
                }
            };

            match self.pop()? {
                None => return Ok(()),
                Some(next) if unsafe { next.len() } > 0 => return Ok(()),
                Some(next) => node = next,
            }
        }
    }

    #[inline]
    fn push(
        &mut self,
        key: K::Read<'k>,
        len: <<<K::Edge as ribbit::Pack>::Packed as edge::Meta>::Key as edge::Key>::Len,
        node: ribbit::Packed<node::Ptr<K::Edge>>,
        edge: &'g Atomic<Edge<K::Edge>>,
    ) {
        // 1 extra byte for node
        self.bits += 8 + len.bits();
        self.history.push(path::Segment {
            key,
            len,
            edge: core::mem::replace(&mut self.edge, NonNull::from(edge)),
            node,
        })
    }

    #[inline]
    fn pop(&mut self) -> Result<Option<ribbit::Packed<node::Ptr<K::Edge>>>, H::PopError> {
        let Some(segment) = self.history.pop()? else {
            return Ok(None);
        };
        self.bits -= segment.len.bits() + 8;
        self.key = segment.key;
        self.edge = segment.edge;
        Ok(Some(segment.node))
    }
}
