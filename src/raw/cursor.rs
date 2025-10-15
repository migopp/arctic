use core::convert::Infallible;
use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::byte;
use crate::edge;
use crate::key;
use crate::node;
use crate::node::Node3;
use crate::raw::Op;
use crate::smr;
use crate::Edge;

/// Stateful traversal over tree.
pub(crate) struct Cursor<'g, 'l, R, H> {
    guard: smr::Guard<'g, 'l>,
    bit: usize,
    key: R,
    root: &'g Atomic128<Edge>,
    history: H,
}

impl<'g, 'l, R: key::Read, H: History<'g, R>> Cursor<'g, 'l, R, H> {
    #[inline]
    pub(crate) fn new(smr: &'l mut smr::Local<'g>, root: &'g Atomic128<Edge>, key: R) -> Self {
        Self {
            guard: smr.protect(key.peek_all()),
            bit: 0,
            key,
            root,
            history: H::default(),
        }
    }

    #[inline]
    pub(crate) fn root(&self) -> &'g Atomic128<Edge> {
        self.root
    }

    #[inline]
    pub(crate) fn bit(&self) -> usize {
        self.bit
    }

    #[inline]
    pub(crate) unsafe fn retire(&mut self, edge: ribbit::Packed<Edge>) {
        unsafe { self.guard.retire(edge) }
    }

    #[inline]
    pub(crate) fn into_guard(self) -> smr::Guard<'g, 'l> {
        self.guard
    }

    #[inline]
    pub(crate) fn traverse_exact(&mut self) -> Option<Result<ribbit::Packed<Edge>, ()>> {
        loop {
            let mut edge = self.root().load_packed(Ordering::Relaxed);

            if edge.is_scan() {
                edge = self.block();
            }

            let meta = edge.meta();

            let save = self.key;
            let len = meta.key().match_exact(&mut self.key)?;

            // Fast path: traversal
            if edge.is_node() {
                let byte = self.key.next()?;
                let node = unsafe { edge.data().into_node_unchecked() };
                let next = node.get(byte)?;
                self.push(save, len, node, next);
                continue;
            }

            self.key = save;

            return if meta.frozen() {
                Some(Err(()))
            } else if meta.leaf() {
                Some(Ok(edge))
            } else {
                validate!(edge.data().is_null());
                None
            };
        }
    }

    /// Return CAS operands to either insert the leaf or structurally update
    /// the tree on the way to inserting the leaf.
    #[inline]
    pub(crate) fn traverse_or_insert(
        &mut self,
        value: u64,
    ) -> Result<(Op, ribbit::Packed<Edge>, ribbit::Packed<Edge>), ()> {
        loop {
            let mut old = self.root().load_packed(Ordering::Relaxed);

            if old.is_scan() {
                old = self.block();
            }

            let old_meta = old.meta();
            let old_data = old.data();
            let save = self.key;
            let r#match = old_meta.key().match_split(&mut self.key);

            // Fast path: traverse
            if let byte::MatchSplit::Full(len) = r#match {
                if old.is_node() {
                    let byte = self.key.next().unwrap();
                    let node = unsafe { old_data.into_node_unchecked() };
                    if let Some(next) = node.get_or_reserve(byte) {
                        self.push(save, len, node, next);
                        continue;
                    }
                }
            }

            if old_meta.frozen() {
                return Err(());
            }

            // Revert key to before the current edge
            self.key = save;

            let (op, new) = match r#match {
                byte::MatchSplit::Full(_) if old.is_node() => {
                    let node = unsafe { old_data.into_node_unchecked() };
                    let (op, new) = node.replace(old);
                    (Op::Node(op), new)
                }
                byte::MatchSplit::Full(_) if self.key.bits() > byte::Len::MAX.bits() as usize => (
                    Op::Edge(edge::Op::Create),
                    Edge::new_node::<Node3, _>(self.key.peek(byte::Len::MAX), false, None),
                ),
                byte::MatchSplit::Full(_) => (
                    Op::Edge(edge::Op::Insert),
                    Edge::new_leaf(
                        self.key
                            .peek(unsafe { byte::Len::from_bits_unchecked(self.key.bits() as u8) }),
                        value,
                    ),
                ),
                byte::MatchSplit::Partial { start, middle, end } => (
                    Op::Edge(edge::Op::Expand),
                    Edge::new_node::<Node3, _>(
                        start,
                        false,
                        Some((middle, old.with_meta(old_meta.with_key(end)))),
                    ),
                ),
            };

            return Ok((op, old, new));
        }
    }

    #[cold]
    fn block(&self) -> ribbit::Packed<Edge> {
        loop {
            core::hint::spin_loop();
            let edge = self.root().load_packed(Ordering::Acquire);
            if !edge.is_scan() {
                return edge;
            }
        }
    }

    #[inline]
    fn push(&mut self, key: R, len: byte::Len, node: node::Ref<'g>, edge: &'g Atomic128<Edge>) {
        // 1 extra byte for node
        self.bit += 8 + len.bits() as usize;
        self.history.push(Segment {
            key,
            len,
            edge: core::mem::replace(&mut self.root, edge),
            node,
        })
    }

    #[cold]
    pub(crate) fn pop(&mut self) -> Result<node::Ref<'g>, H::PopError> {
        let segment = self.history.pop()?.expect("Root edge can never be frozen");
        self.bit -= segment.len.bits() as usize + 8;
        self.key = segment.key;
        self.root = segment.edge;
        Ok(segment.node)
    }
}

impl<'g, 'l, R: key::Read> Cursor<'g, 'l, R, Optimistic<R>> {
    pub(crate) fn traverse_prefix(&mut self) -> Option<ribbit::Packed<Edge>> {
        loop {
            let edge = self.root.load_packed(Ordering::Acquire);
            let meta = edge.meta();
            let data = edge.data();

            match meta.key().match_prefix(&mut self.key)? {
                byte::MatchPrefix::Full(len) if edge.is_node() => {
                    let node = unsafe { data.into_node_unchecked() };
                    let Some(byte) = self.key.next() else {
                        return Some(edge);
                    };
                    self.root = node.get(byte)?;
                    self.bit += len.bits() as usize + 8;
                }
                byte::MatchPrefix::Full(_) | byte::MatchPrefix::Partial => return Some(edge),
            }
        }
    }

    #[inline]
    pub(crate) fn traverse_value(mut self) -> Option<u64> {
        loop {
            let edge = self.root.load_packed(Ordering::Relaxed);
            let meta = edge.meta();

            let _ = meta.key().match_exact(&mut self.key)?;
            let data = edge.data();

            if meta.leaf() {
                return Some(data.into_leaf());
            } else if data.is_null() {
                return None;
            } else {
                let byte = self.key.next()?;
                let data = edge.data();
                let node = unsafe { data.into_node_unchecked() };
                self.root = node.get(byte)?;
            }
        }
    }
}

pub(crate) trait History<'a, K>: Default {
    type PopError;

    fn push(&mut self, segment: Segment<'a, K>);
    fn pop(&mut self) -> Result<Option<Segment<'a, K>>, Self::PopError>;
}

pub(crate) struct Optimistic<K>(PhantomData<K>);

impl<R> Default for Optimistic<R> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<'a, R> History<'a, R> for Optimistic<R> {
    type PopError = ();

    #[inline]
    fn push(&mut self, _segment: Segment<'a, R>) {}

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'a, R>>, Self::PopError> {
        Err(())
    }
}

pub(crate) struct Pessimistic<'a, R> {
    path: Vec<Segment<'a, R>>,
}

impl<R> Default for Pessimistic<'_, R> {
    fn default() -> Self {
        Self {
            path: Vec::default(),
        }
    }
}

impl<'a, R> History<'a, R> for Pessimistic<'a, R> {
    type PopError = Infallible;

    #[inline]
    fn push(&mut self, segment: Segment<'a, R>) {
        self.path.push(segment);
    }

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'a, R>>, Self::PopError> {
        Ok(self.path.pop())
    }
}

/// Path segment consists of:
/// - Current key before matching on edge
/// - Number of bytes matched along edge
/// - Edge to match next
/// - Node underneath edge
pub(crate) struct Segment<'a, R> {
    key: R,
    len: byte::Len,
    edge: &'a Atomic128<Edge>,
    node: node::Ref<'a>,
}
