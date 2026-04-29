pub(crate) mod path;
pub(crate) use path::Path;

use core::marker::PhantomData;
use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use ribbit::Atomic;

use crate::raw::Edge;
use crate::raw::Frozen;
use crate::raw::edge;
use crate::raw::edge::Key as _;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Len as _;
use crate::raw::node;
use crate::raw::node::Node3;
use crate::stat;

pub(crate) struct CursorMut<'g, R: key::Read>(Cursor<'g, R, path::Discard>);

impl<'g, R: key::Read> CursorMut<'g, R> {
    #[inline]
    pub(crate) fn new(root: &'g mut Atomic<Edge<R::Edge>>, key: R) -> Self {
        Self(unsafe { Cursor::new(root, key) })
    }

    #[inline]
    pub(crate) fn edge_mut(&mut self) -> &'g mut Atomic<Edge<R::Edge>> {
        unsafe { self.0.edge.as_mut() }
    }
}

impl<'g, R: key::Read> core::ops::Deref for CursorMut<'g, R> {
    type Target = Cursor<'g, R, path::Discard>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'g, R: key::Read> core::ops::DerefMut for CursorMut<'g, R> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Tree traversal state.
pub(crate) struct Cursor<'g, R: key::Read, P> {
    len: R::Len,

    /// Current key reader
    reader: R,

    /// Edge this cursor currently points to
    edge: NonNull<Atomic<Edge<R::Edge>>>,

    /// Path this cursor has taken
    path: P,

    _global: PhantomData<&'g Atomic<Edge<R::Edge>>>,
}

pub(crate) enum Insert<E: ribbit::Pack<Packed: edge::Meta>> {
    Value {
        old_value: Option<u64>,
        old: ribbit::Packed<Edge<E>>,
        key: <E::Packed as edge::Meta>::Key,
    },

    Replace {
        old_node: ribbit::Packed<node::Ptr<E>>,
        old: ribbit::Packed<Edge<E>>,
    },
}

impl<'g, R, P> Cursor<'g, R, P>
where
    R: key::Read,
    P: Path<R>,
{
    /// # Safety
    ///
    /// Caller must ensure that all nodes underneath `root` along the path associated
    /// with `reader` live at least as long as this struct.
    #[inline]
    pub(crate) unsafe fn new(root: &'g Atomic<Edge<R::Edge>>, reader: R) -> Self {
        Self {
            len: R::Len::ZERO,
            edge: NonNull::from(root),
            reader,
            path: P::default(),
            _global: PhantomData,
        }
    }

    #[inline]
    pub(crate) fn edge(&self) -> &'g Atomic<Edge<R::Edge>> {
        unsafe { self.edge.as_ref() }
    }

    #[inline]
    pub(crate) fn len(&self) -> R::Len {
        self.len
    }

    /// Traverse to the value associated with the key, if it exists.
    #[inline]
    pub(crate) fn traverse_get(&mut self) -> Option<u64> {
        loop {
            let edge = self.edge().load_packed(Ordering::Acquire);

            let _ = self.reader.match_exact(edge.meta())?;

            match edge.child()? {
                edge::Child::Node(node) => {
                    let byte = if const { R::LEN.is_none() } {
                        self.reader
                            .next()
                            .expect("Precondition: no key is prefix of another key")
                    } else {
                        unsafe { self.reader.next_unchecked() }
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
    pub(crate) fn traverse_prefix(&mut self) -> Option<ribbit::Packed<Edge<R::Edge>>> {
        loop {
            let edge = self.edge().load_packed(Ordering::Acquire);
            let meta = edge.meta();

            let reader = self.reader;
            let (key, exact) = self.reader.match_inexact(meta);

            if exact {
                if let Some(node) = edge.as_node() {
                    if let Some(byte) = self.reader.next() {
                        let next = unsafe { node.get(byte) }?;
                        self.push(reader, key.len(), node, next);
                        continue;
                    }
                }
            }

            self.reader = reader;

            if edge.is_null() || meta.key().prefix(key.len()) != key {
                return None;
            } else {
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
    ) -> Option<Result<ribbit::Packed<Edge<R::Edge>>, Frozen>> {
        loop {
            let edge = self.edge().load_packed(Ordering::Acquire);
            let meta = edge.meta();

            let reader = self.reader;
            let len = self.reader.match_exact(meta)?;

            if let Some(node) = edge.as_node() {
                let byte = if const { R::LEN.is_none() } {
                    self.reader
                        .next()
                        .expect("Precondition: no key is prefix of another key")
                } else {
                    unsafe { self.reader.next_unchecked() }
                };

                let next = unsafe { node.get(byte) }?;
                self.push(reader, len, node, next);
                continue;
            }

            self.reader = reader;

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
    #[inline]
    pub(crate) fn traverse_insert(&mut self) -> Insert<R::Edge> {
        loop {
            let old = self.edge().load_packed(Ordering::Acquire);

            let reader = self.reader;
            let (key, exact) = self.reader.match_inexact(old.meta());

            if exact {
                if let Some(node) = old.as_node() {
                    let byte = if const { R::LEN.is_none() } {
                        self.reader
                            .next()
                            .expect("Precondition: no key is prefix of another key")
                    } else {
                        unsafe { self.reader.next_unchecked() }
                    };

                    if let Some(next) = unsafe { node.get_or_insert(byte) } {
                        self.push(reader, key.len(), node, next);
                        continue;
                    }
                }
            }

            // Revert key to before the current edge
            self.reader = reader;

            let old_value = match (exact, old.child()) {
                // Edge expansion
                (false, _)
                // Node creation or insertion
                | (true, None) => None,
                // Update
                (true, Some(edge::Child::Value(value))) => Some(value),
                // Node replacement
                (true, Some(edge::Child::Node(node))) => {
                    return Insert::Replace {
                        old_node: node,
                        old,
                    };
                }
            };

            return Insert::Value {
                old_value,
                old,
                key,
            };
        }
    }

    /// Locally create a path from the current edge
    /// to insert this key value pair. May create nodes recursively if
    /// the remaining key is long.
    pub(crate) fn create_path(
        &self,
        old: ribbit::Packed<Edge<R::Edge>>,
        key: <<R::Edge as ribbit::Pack>::Packed as edge::Meta>::Key,
        value: u64,
    ) -> Result<ribbit::Packed<Edge<R::Edge>>, Frozen> {
        if old.meta().is_frozen() {
            return Err(Frozen);
        }

        let mut reader = self.reader;

        let new = match old.meta().expand(key) {
            Err(_) => Edge::new_path(reader, value),
            Ok((start, middle, end)) => {
                let start_ = reader.read(start.len());
                validate_eq!(start, start_);

                let byte = unsafe { reader.next_unchecked() };
                validate_ne!(middle, byte);

                // NOTE: must put new allocation first because
                // `deallocate_recursive` recurses on first edge
                Node3::new_expand(
                    start,
                    [byte, middle],
                    [Edge::new_path(reader, value), old.with_meta(end)],
                )
            }
        };

        Ok(new)
    }

    /// Freeze and replace the node containing `self.edge`.
    ///
    /// Returns `Err(_)` if the path does not support popping,
    /// `Ok(Some(node))` if this thread successfully replaced `node`,
    /// or `Ok(None)` if this thread did not replace the node (e.g.,
    /// another thread won the CAS race or an edge expansion SMO pushed
    /// down the frozen node).
    #[cold]
    pub(crate) fn freeze(
        &mut self,
    ) -> Result<Option<ribbit::Packed<node::Ptr<R::Edge>>>, P::PopError> {
        let mut node = self.pop()?.expect("Root edge cannot be frozen");
        let mut edge = self.edge().load_packed(Ordering::Acquire);
        let mut pop = 1;

        let old = loop {
            while edge.meta().is_frozen() {
                node = self.pop()?.expect("Root edge cannot be frozen");
                edge = self.edge().load_packed(Ordering::Acquire);
                pop += 1;
            }

            let meta = edge.meta();

            let old = match edge.child() {
                Some(edge::Child::Node(old)) if old == node => old,
                // Child has changed since we last traversed
                // Optimistically assume that node replacement was completed by a different thread
                None | Some(_) => break None,
            };

            let (op, new) = unsafe { node.replace::<true>(meta) };

            match self.edge().compare_exchange_packed(
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
    fn push(
        &mut self,
        key: R,
        len: <<<R::Edge as ribbit::Pack>::Packed as edge::Meta>::Key as edge::Key>::Len,
        node: ribbit::Packed<node::Ptr<R::Edge>>,
        edge: &'g Atomic<Edge<R::Edge>>,
    ) {
        // 1 extra byte for node
        self.len += R::Len::BYTE + len;
        self.path.push(path::Segment {
            reader: key,
            len,
            edge: core::mem::replace(&mut self.edge, NonNull::from(edge)),
            node,
        })
    }

    #[inline]
    pub(crate) fn pop(
        &mut self,
    ) -> Result<Option<ribbit::Packed<node::Ptr<R::Edge>>>, P::PopError> {
        let Some(segment) = self.path.pop()? else {
            return Ok(None);
        };
        self.len -= R::Len::BYTE + segment.len;
        self.reader = segment.reader;
        self.edge = segment.edge;
        Ok(Some(segment.node))
    }

    #[inline]
    pub(crate) fn trim(&mut self, len: R::Len) {
        self.path.trim(len);
        self.reader.trim(len);
    }
}
