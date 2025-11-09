pub(crate) mod path;

use core::sync::atomic::Ordering;

use path::History as _;
use ribbit::atomic::Atomic128;

use crate::key;
use crate::raw::edge;
use crate::raw::node;
use crate::raw::node::Node3;
use crate::raw::Edge;
use crate::raw::Op;
use crate::stat;

/// Tree traversal state.
pub(crate) struct Point<'g, R, M, H> {
    /// Total number of bits read from `key`
    bits: usize,

    /// Current key reader
    key: R,

    /// Edge this cursor currently points to
    edge: &'g Atomic128<Edge<M>>,

    /// Path history of this cursor (sequence of path segments)
    history: H,
}

impl<'g, R, M, H> Point<'g, R, M, H>
where
    R: key::Read<Edge = M>,
    M: edge::Meta,
    H: path::History<'g, R, M>,
{
    #[inline]
    pub(crate) unsafe fn new(root: &'g Atomic128<Edge<M>>, key: R) -> Self {
        Self {
            bits: 0,
            edge: root,
            key,
            history: H::new(root, key),
        }
    }

    #[inline]
    pub(crate) fn edge(&self) -> &'g Atomic128<Edge<M>> {
        self.edge
    }

    #[inline]
    pub(crate) fn bits(&self) -> usize {
        self.bits
    }

    #[inline]
    pub(crate) fn traverse_exact(&mut self) -> Option<Result<ribbit::Packed<Edge<M>>, ()>> {
        loop {
            let edge = self.edge.load_packed(Ordering::Relaxed);
            let meta = edge.meta();
            let save = self.key;
            let bits = self.key.read_exact(meta)?;

            // Fast path: traversal
            if let Some(node) = edge.as_node() {
                if node.scan() {
                    match self.wait_for_scan(stat::Counter::ScanUpdate) {
                        Ok(_) => continue,
                        Err(()) => return Some(Err(())),
                    }
                }

                let byte = self.key.next()?;
                let node = unsafe { node.into_ref_unchecked() };
                let next = node.get(byte)?;
                self.push(save, bits, node, next);
                continue;
            }

            self.key = save;

            return if M::is_frozen(meta) {
                Some(Err(()))
            } else if M::is_value(meta) {
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
    pub(crate) fn traverse_or_insert(
        &mut self,
        value: u64,
    ) -> Result<(Op, ribbit::Packed<Edge<M>>, ribbit::Packed<Edge<M>>), ()> {
        loop {
            let old = self.edge.load_packed(Ordering::Relaxed);
            let old_meta = old.meta();
            let mut save = self.key;

            let key = self.key.read_inexact(old_meta);

            // Fast path: traverse
            if M::equal(old_meta, key) {
                if let Some(node) = old.as_node() {
                    if node.scan() {
                        self.wait_for_scan(stat::Counter::ScanInsert)?;
                        continue;
                    }

                    let byte = self.key.next().unwrap();
                    let node = unsafe { node.into_ref_unchecked() };
                    if let Some(next) = node.get_or_reserve(byte) {
                        self.push(save, M::bits(key), node, next);
                        continue;
                    }
                }
            }

            if M::is_frozen(old_meta) {
                return Err(());
            }

            // Revert key to before the current edge
            self.key = save;

            let (op, new) = match M::expand(old_meta, key) {
                Err(_) => match old.child() {
                    Some(edge::Child::Node(node)) => {
                        let node = unsafe { node.into_ref_unchecked() };
                        let (op, new) = node.replace(old_meta);
                        (Op::Node(op), new)
                    }
                    None | Some(edge::Child::Value(_)) => {
                        // Note: avoid mutating `self.key` here
                        let meta = save.read_all();
                        if save.bits() == 0 {
                            (Op::Edge(edge::Op::Insert), Edge::new_value(meta, value))
                        } else {
                            (
                                Op::Edge(edge::Op::Create),
                                Edge::new_node::<Node3<M>, _>(meta, None),
                            )
                        }
                    }
                },
                Ok((start, middle, end)) => (
                    Op::Edge(edge::Op::Expand),
                    Edge::new_node::<Node3<M>, _>(start, [(middle, old.with_meta(end))]),
                ),
            };

            return Ok((op, old, new));
        }
    }

    #[cold]
    pub(crate) fn freeze(&mut self) -> Result<Option<ribbit::Packed<Edge<M>>>, H::PopError> {
        let mut node = self.pop()?;
        let mut edge = self.edge.load_packed(Ordering::Acquire);

        loop {
            while M::is_frozen(edge.meta()) {
                node = self.pop()?;
                edge = self.edge.load_packed(Ordering::Acquire);
            }

            let meta = edge.meta();

            let old = match edge.child() {
                Some(edge::Child::Node(old)) if old.is_ref(node) => old,
                // Already helped by another thread
                None | Some(edge::Child::Node(_)) => return Ok(None),
                // Should be impossible to freeze value
                Some(edge::Child::Value(_)) => unreachable!(),
            };

            let (op, new) = node.replace(meta);

            match self.edge.compare_exchange_packed(
                edge,
                // FIXME: shouldn't need to unwrap here
                new.with_node(unsafe { new.as_node().unwrap_unchecked() }.with_scan(old.scan())),
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    stat::increment(op);
                    return Ok(Some(edge));
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
        }
    }

    #[cold]
    pub(crate) fn wait_for_scan(
        &self,
        counter: stat::Counter,
    ) -> Result<ribbit::Packed<Edge<M>>, ()> {
        stat::increment(counter);

        loop {
            core::hint::spin_loop();
            let edge = self.edge.load_packed(Ordering::Acquire);

            match edge.child() {
                None => return Ok(edge),
                Some(edge::Child::Value(_)) => unreachable!(),
                Some(edge::Child::Node(node)) if !node.scan() => return Ok(edge),
                Some(edge::Child::Node(_)) if M::is_frozen(edge.meta()) => return Err(()),
                Some(edge::Child::Node(_)) => continue,
            }
        }
    }

    #[inline]
    fn push(&mut self, key: R, bits: usize, node: node::Ref<'g, M>, edge: &'g Atomic128<Edge<M>>) {
        // 1 extra byte for node
        self.bits += 8 + bits;
        self.history.push(path::Segment {
            key,
            bits,
            edge: core::mem::replace(&mut self.edge, edge),
            node,
        })
    }

    #[cold]
    fn pop(&mut self) -> Result<node::Ref<'g, M>, H::PopError> {
        let segment = self.history.pop()?.expect("Root edge can never be frozen");
        self.bits -= segment.bits + 8;
        self.key = segment.key;
        self.edge = segment.edge;
        Ok(segment.node)
    }
}

impl<'g, R, M> Point<'g, R, M, path::Discard>
where
    R: key::Read<Edge = M>,
    M: edge::Meta,
{
    #[inline]
    pub(crate) unsafe fn get(root: &'g Atomic128<Edge<M>>, key: R) -> Option<u64>
    where
        R: key::Read,
    {
        let mut cursor = Self::new(root, key);
        loop {
            let edge = cursor.edge.load_packed(Ordering::Relaxed);
            let meta = edge.meta();
            let _ = cursor.key.read_exact(meta)?;

            match edge.child()? {
                edge::Child::Node(node) => {
                    let byte = cursor.key.next()?;
                    let node = unsafe { node.into_ref_unchecked() };
                    cursor.edge = node.get(byte)?;
                }
                edge::Child::Value(value) => {
                    return Some(value);
                }
            }
        }
    }
}

pub(crate) struct Prefix<'g, R, M, H> {
    prefix: R,
    cursor: Point<'g, R, M, H>,
}

impl<'g, R, M, H> Prefix<'g, R, M, H>
where
    R: key::Read<Edge = M>,
    M: edge::Meta,
    H: path::History<'g, R, M>,
{
    pub(crate) unsafe fn new_root(root: &'g Atomic128<Edge<M>>) -> Self {
        let prefix = R::default();
        Self {
            prefix,
            cursor: Point::new(root, prefix),
        }
    }

    pub(crate) unsafe fn new(root: &'g Atomic128<Edge<M>>, prefix: R) -> Option<Self> {
        let mut cursor = Self {
            prefix,
            cursor: Point::new(root, prefix),
        };
        cursor.traverse()?;
        Some(cursor)
    }

    pub(crate) fn prefix(&self) -> R {
        self.prefix.slice(self.cursor.bits)
    }

    pub(crate) fn traverse(&mut self) -> Option<ribbit::Packed<Edge<M>>> {
        let (key, edge) = loop {
            let edge = self.cursor.edge.load_packed(Ordering::Acquire);
            let meta = edge.meta();
            let save = self.cursor.key;

            match self.cursor.key.read_prefix(meta)? {
                bits if bits == M::bits(meta) => {
                    if let Some(node) = edge.as_node() {
                        let node = unsafe { node.into_ref_unchecked() };
                        let Some(byte) = self.cursor.key.next() else {
                            break (save, edge);
                        };
                        let next = node.get(byte)?;
                        self.cursor.push(save, bits, node, next);
                        continue;
                    }
                }
                _ => (),
            }

            if edge.is_null() {
                return None;
            } else {
                break (save, edge);
            }
        };

        self.cursor.key = key;
        Some(edge)
    }
}

impl<'g, R, M> Prefix<'g, R, M, path::Hybrid<'g, R, M>>
where
    R: key::Read<Edge = M>,
    M: edge::Meta,
{
    #[cold]
    pub(crate) fn freeze(&mut self) -> Option<ribbit::Packed<Edge<M>>> {
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

impl<'g, R, C, H> core::ops::Deref for Prefix<'g, R, C, H> {
    type Target = Point<'g, R, C, H>;
    fn deref(&self) -> &Self::Target {
        &self.cursor
    }
}

impl<'g, R, C, H> core::ops::DerefMut for Prefix<'g, R, C, H> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.cursor
    }
}
