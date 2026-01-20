pub(crate) mod path;

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
use crate::raw::Key;
use crate::raw::Smo;
use crate::stat;

/// Tree traversal state.
pub(crate) struct Cursor<'k, 'g, K: Key, H> {
    /// Total number of bits read from `key`
    bits: usize,

    /// Current key reader
    key: K::Read<'k>,

    /// Edge this cursor currently points to
    edge: &'g Atomic<Edge<K::Edge>>,

    /// Path history of this cursor (sequence of path segments)
    history: H,
}

// pub(crate) enum Insert<E: ribbit::Pack<Packed: edge::Meta>> {
//     Value {
//         old: ribbit::Packed<Edge<E>>,
//         key: <E::Packed as edge::Meta>::Key,
//     },
//
//     /// Structural modification required
//     Smo {
//         op: Smo,
//         old: ribbit::Packed<Edge<E>>,
//         new: ribbit::Packed<Edge<E>>,
//     },
//     Frozen,
// }

impl<'k, 'g, K, H> Cursor<'k, 'g, K, H>
where
    K: Key,
    H: path::History<'k, 'g, K>,
{
    #[inline]
    pub(crate) unsafe fn new(root: &'g Atomic<Edge<K::Edge>>, key: K::Read<'k>) -> Self {
        Self {
            bits: 0,
            edge: root,
            key,
            history: H::new(root, key),
        }
    }

    #[inline]
    pub(crate) fn edge(&self) -> &'g Atomic<Edge<K::Edge>> {
        self.edge
    }

    #[inline]
    pub(crate) fn bits(&self) -> usize {
        self.bits
    }

    #[inline]
    pub(crate) unsafe fn traverse_get(mut self) -> Option<u64> {
        loop {
            let edge = self.edge.load_packed(Ordering::Relaxed);

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

                    self.edge = unsafe { node.get(byte) }?;
                }
                edge::Child::Value(value) => {
                    return Some(value);
                }
            }
        }
    }

    #[inline]
    pub(crate) fn traverse_update(&mut self) -> Option<Result<ribbit::Packed<Edge<K::Edge>>, ()>> {
        loop {
            let edge = self.edge.load_packed(Ordering::Relaxed);
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
                Some(Err(()))
            } else if meta.is_value() {
                Some(Ok(edge))
            } else {
                validate!(edge.is_null());
                None
            };
        }
    }

    /// Return CAS operands to either insert the value or structurally update
    /// the tree on the way to inserting the value.
    #[inline]
    pub(crate) fn traverse_upsert(
        &mut self,
        value: u64,
    ) -> Result<
        (
            Smo,
            ribbit::Packed<Edge<K::Edge>>,
            ribbit::Packed<Edge<K::Edge>>,
        ),
        (),
    > {
        loop {
            let old = self.edge.load_packed(Ordering::Relaxed);
            let old_meta = old.meta();
            let mut save = self.key;

            let (key, exact) = self.key.match_inexact(old_meta);

            // Fast path: traverse
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

            if old_meta.is_frozen() {
                return Err(());
            }

            let (op, new) = match old_meta.expand(key) {
                Err(_) => match old.child() {
                    Some(edge::Child::Node(node)) => unsafe { node.replace(old_meta) },
                    None | Some(edge::Child::Value(_)) => {
                        // Note: avoid mutating `self.key` here
                        (Smo::CreateNode, Edge::new_path(save, value))
                    }
                },
                Ok((start, middle, end)) => {
                    let _ = save.read(start.len());
                    let byte = unsafe { save.next_unchecked() };
                    (
                        Smo::ExpandEdge,
                        Edge::new_node::<Node3<K::Edge>, _, _>(
                            start,
                            [byte, middle],
                            [Edge::new_path(save, value), old.with_meta(end)],
                        ),
                    )
                }
            };

            return Ok((op, old, new));
        }
    }

    pub(crate) fn traverse_prefix(&mut self) -> Option<ribbit::Packed<Edge<K::Edge>>> {
        loop {
            let edge = self.edge.load_packed(Ordering::Acquire);
            let meta = edge.meta();
            let save = self.key;

            let (key, exact) = self.key.match_inexact(meta);

            // Full match
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

    #[cold]
    pub(crate) fn freeze(&mut self) -> Result<Option<ribbit::Packed<Edge<K::Edge>>>, H::PopError> {
        let mut node = self.pop()?;
        let mut edge = self.edge.load_packed(Ordering::Acquire);
        let mut pop = 1;

        let edge = loop {
            while edge.meta().is_frozen() {
                node = self.pop()?;
                edge = self.edge.load_packed(Ordering::Acquire);
                pop += 1;
            }

            let meta = edge.meta();

            match edge.child() {
                Some(edge::Child::Node(old)) if old == node => (),
                // Already helped by another thread OR freeze was pushed down by
                // a concurrent edge expansion operation
                None | Some(_) => break None,
            }

            let (op, new) = unsafe { node.replace(meta) };

            match self.edge.compare_exchange_packed(
                edge,
                // FIXME: shouldn't need to unwrap here
                new.with_node(unsafe { new.as_node().unwrap_unchecked() }),
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    stat::increment(op);
                    break Some(edge);
                }
                Err(conflict) => {
                    if op.is_allocate() {
                        if let Some(edge::Child::Node(node)) = new.child() {
                            unsafe {
                                node.deallocate(stat::Counter::FreeConflict);
                            }
                        }
                    }
                    edge = conflict;
                }
            };
        };

        stat::record(stat::Record::FreezePop, pop);
        Ok(edge)
    }

    #[inline]
    fn push(
        &mut self,
        key: K::Read<'k>,
        len: <<K::Edge as ribbit::Pack>::Packed as edge::Meta>::Len,
        node: ribbit::Packed<node::Ptr<K::Edge>>,
        edge: &'g Atomic<Edge<K::Edge>>,
    ) {
        // 1 extra byte for node
        self.bits += 8 + len.bits();
        self.history.push(path::Segment {
            key,
            len,
            edge: core::mem::replace(&mut self.edge, edge),
            node,
        })
    }

    #[cold]
    fn pop(&mut self) -> Result<ribbit::Packed<node::Ptr<K::Edge>>, H::PopError> {
        let segment = self.history.pop()?.expect("Root edge can never be frozen");
        self.bits -= segment.len.bits() + 8;
        self.key = segment.key;
        self.edge = segment.edge;
        Ok(segment.node)
    }
}
