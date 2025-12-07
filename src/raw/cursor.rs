pub(crate) mod path;

use core::sync::atomic::Ordering;

use path::History as _;
use ribbit::Atomic;

use crate::raw::edge;
use crate::raw::edge::Key as _;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key::Read as _;
use crate::raw::node::Node3;
use crate::raw::Edge;
use crate::raw::Key;
use crate::raw::Smo;
use crate::stat;

/// Tree traversal state.
pub(crate) struct Point<'k, 'g, K: Key, H> {
    /// Total number of bits read from `key`
    bits: usize,

    /// Current key reader
    key: K::Read<'k>,

    /// Edge this cursor currently points to
    edge: &'g Atomic<Edge<K::Edge>>,

    /// Path history of this cursor (sequence of path segments)
    history: H,
}

pub(crate) enum Insert<E: ribbit::Pack<Packed: edge::Meta>> {
    Value {
        old: ribbit::Packed<Edge<E>>,
        key: <E::Packed as edge::Meta>::Key,
    },

    /// Structural modification required
    Smo {
        op: Smo,
        old: ribbit::Packed<Edge<E>>,
        new: ribbit::Packed<Edge<E>>,
    },
    Frozen,
}

impl<'k, 'g, K, H> Point<'k, 'g, K, H>
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
    pub(crate) fn traverse_exact(&mut self) -> Option<Result<ribbit::Packed<Edge<K::Edge>>, ()>> {
        loop {
            let edge = self.edge.load_packed(Ordering::Relaxed);
            let meta = edge.meta();
            let save = self.key;

            // Fast path: traversal
            let len = self.key.read_exact(meta)?;

            if let Some(node) = edge.as_node() {
                if node.scan() {
                    match self.wait_for_scan(stat::Counter::ScanUpdate) {
                        Ok(_) => continue,
                        Err(()) => return Some(Err(())),
                    }
                }

                let byte = if cfg!(feature = "validate") {
                    self.key
                        .next()
                        .expect("Precondition: no key is prefix of another key")
                } else {
                    unsafe { self.key.next_unchecked() }
                };

                let next = unsafe { node.get_unchecked(byte) }?;
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
    pub(crate) fn traverse_or_insert(&mut self) -> Insert<K::Edge> {
        loop {
            let old = self.edge.load_packed(Ordering::Relaxed);
            let old_meta = old.meta();
            let mut save = self.key;

            let old_key = old_meta.key();
            let old_len = old_key.len();
            let key = self.key.read(old_len);

            // Fast path: traverse
            if key == old_key {
                if let Some(node) = old.as_node() {
                    if node.scan() {
                        match self.wait_for_scan(stat::Counter::ScanInsert) {
                            Ok(_) => continue,
                            Err(()) => return Insert::Frozen,
                        }
                    }

                    let byte = if cfg!(feature = "validate") {
                        self.key
                            .next()
                            .expect("Precondition: no key is prefix of another key")
                    } else {
                        unsafe { self.key.next_unchecked() }
                    };

                    if let Some(next) = unsafe { node.get_or_insert_unchecked(byte) } {
                        self.push(save, old_len, node, next);
                        continue;
                    }
                }
            }

            // Revert key to before the current edge
            self.key = save;

            let (op, new) = match old_meta.expand(key) {
                Err(_) => match old.child() {
                    Some(edge::Child::Node(_)) if old_meta.is_frozen() => return Insert::Frozen,
                    Some(edge::Child::Node(node)) => {
                        let (op, new) = unsafe { node.replace_unchecked(old_meta) };
                        (Smo::Node(op), new)
                    }
                    None | Some(edge::Child::Value(_)) => {
                        // Note: avoid mutating `self.key` here
                        let key =
                            save.read(<<K::Edge as ribbit::Pack>::Packed as edge::Meta>::MAX_LEN);

                        if save.bits() == 0 {
                            return Insert::Value { old, key };
                        }

                        if old_meta.is_frozen() {
                            return Insert::Frozen;
                        }

                        (
                            Smo::Edge(edge::Smo::Create),
                            Edge::new_node::<Node3<K::Edge>, _, _>(key, [], []),
                        )
                    }
                },
                Ok(_) if old_meta.is_frozen() => return Insert::Frozen,
                Ok((start, middle, end)) => (
                    Smo::Edge(edge::Smo::Expand),
                    Edge::new_node::<Node3<K::Edge>, _, _>(start, [middle], [old.with_meta(end)]),
                ),
            };

            return Insert::Smo { op, old, new };
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

            let old = match edge.child() {
                Some(edge::Child::Node(old)) if old == node => old,
                // Already helped by another thread OR freeze was pushed down by
                // a concurrent edge expansion operation
                None | Some(_) => break None,
            };

            let (op, new) = unsafe { node.replace_unchecked(meta) };

            match self.edge.compare_exchange_packed(
                edge,
                // FIXME: shouldn't need to unwrap here
                new.with_node(unsafe { new.as_node().unwrap_unchecked() }.with_scan(old.scan())),
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
                                node.deallocate_unchecked(stat::Counter::FreeConflict);
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

    #[cold]
    pub(crate) fn wait_for_scan(
        &self,
        counter: stat::Counter,
    ) -> Result<ribbit::Packed<Edge<K::Edge>>, ()> {
        stat::increment(counter);

        loop {
            core::hint::spin_loop();
            let edge = self.edge.load_packed(Ordering::Acquire);

            match edge.child() {
                None => return Ok(edge),
                Some(edge::Child::Value(_)) => unreachable!(),
                Some(edge::Child::Node(node)) if !node.scan() => return Ok(edge),
                Some(edge::Child::Node(_)) if edge.meta().is_frozen() => return Err(()),
                Some(edge::Child::Node(_)) => continue,
            }
        }
    }

    #[inline]
    fn push(
        &mut self,
        key: K::Read<'k>,
        len: <<K::Edge as ribbit::Pack>::Packed as edge::Meta>::Len,
        node: ribbit::Packed<edge::Node<K::Edge>>,
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
    fn pop(&mut self) -> Result<ribbit::Packed<edge::Node<K::Edge>>, H::PopError> {
        let segment = self.history.pop()?.expect("Root edge can never be frozen");
        self.bits -= segment.len.bits() + 8;
        self.key = segment.key;
        self.edge = segment.edge;
        Ok(segment.node)
    }
}

impl<'k, 'g, K> Point<'k, 'g, K, path::Discard>
where
    K: Key,
{
    #[inline]
    pub(crate) unsafe fn get(root: &'g Atomic<Edge<K::Edge>>, key: K::Read<'k>) -> Option<u64> {
        let mut cursor = Self::new(root, key);
        loop {
            let edge = cursor.edge.load_packed(Ordering::Relaxed);

            let _ = cursor.key.read_exact(edge.meta())?;

            match edge.child()? {
                edge::Child::Node(node) => {
                    let byte = if cfg!(feature = "validate") {
                        cursor
                            .key
                            .next()
                            .expect("Precondition: no key is prefix of another key")
                    } else {
                        unsafe { cursor.key.next_unchecked() }
                    };

                    cursor.edge = unsafe { node.get_unchecked(byte) }?;
                }
                edge::Child::Value(value) => {
                    return Some(value);
                }
            }
        }
    }
}

pub(crate) struct Prefix<'k, 'g, K: Key, H> {
    prefix: K::Read<'k>,
    cursor: Point<'k, 'g, K, H>,
}

impl<'k, 'g, K, H> Prefix<'k, 'g, K, H>
where
    K: Key,
    H: path::History<'k, 'g, K>,
{
    pub(crate) unsafe fn new_root(root: &'g Atomic<Edge<K::Edge>>) -> Self {
        let prefix = K::Read::default();
        Self {
            prefix,
            cursor: Point::new(root, prefix),
        }
    }

    pub(crate) unsafe fn new(root: &'g Atomic<Edge<K::Edge>>, prefix: K::Read<'k>) -> Option<Self> {
        let mut cursor = Self {
            prefix,
            cursor: Point::new(root, prefix),
        };
        cursor.traverse()?;
        Some(cursor)
    }

    pub(crate) fn prefix(&self) -> K::Read<'k> {
        self.prefix.prefix(self.cursor.bits)
    }

    pub(crate) fn traverse(&mut self) -> Option<ribbit::Packed<Edge<K::Edge>>> {
        loop {
            let edge = self.cursor.edge.load_packed(Ordering::Acquire);
            let meta = edge.meta();
            let save = self.cursor.key;

            let key_edge = meta.key();
            let len_edge = key_edge.len();
            let key_cursor = self.cursor.key.read(len_edge);

            // Full match
            if key_edge == key_cursor {
                if let Some(node) = edge.as_node() {
                    if let Some(byte) = self.cursor.key.next() {
                        let next = unsafe { node.get_unchecked(byte) }?;
                        self.cursor.push(save, len_edge, node, next);
                        continue;
                    }
                }
            }

            if edge.is_null() || key_cursor != key_edge.prefix(key_cursor.len()) {
                return None;
            } else {
                self.cursor.key = save;
                return Some(edge);
            }
        }
    }
}

impl<'k, 'g, K> Prefix<'k, 'g, K, path::Hybrid<'k, 'g, K>>
where
    K: Key,
{
    #[cold]
    pub(crate) fn freeze(&mut self) -> Option<ribbit::Packed<Edge<K::Edge>>> {
        todo!()
        // match self.cursor.freeze() {
        //     Ok(()) => return self.traverse(),
        //     Err(()) => (),
        // }
        //
        // self.upgrade();
        // self.traverse()?;
        // match self.cursor.freeze() {
        //     Ok(()) => self.traverse(),
        //     Err(()) => unreachable!(),
        // }
    }

    #[expect(unused)]
    #[cold]
    fn upgrade(&mut self) {
        let root = match self.history {
            path::Hybrid::Discard { root } => root,
            path::Hybrid::Retain { .. } => return,
        };

        self.edge = root;
        self.key = self.prefix;
        self.bits = 0;
        self.history = path::Hybrid::Retain(path::Retain::new(root, self.key));
    }
}

impl<'k, 'g, K, H> core::ops::Deref for Prefix<'k, 'g, K, H>
where
    K: Key,
{
    type Target = Point<'k, 'g, K, H>;
    fn deref(&self) -> &Self::Target {
        &self.cursor
    }
}

impl<'k, 'g, K, H> core::ops::DerefMut for Prefix<'k, 'g, K, H>
where
    K: Key,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.cursor
    }
}
