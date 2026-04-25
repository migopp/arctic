use core::fmt::Debug;
use core::marker::PhantomData;
use core::num::NonZeroU32;
use core::num::NonZeroU64;
use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use ribbit::Atomic;
use ribbit::OptionExt as _;
use ribbit::Pack as _;

mod iter;
mod linear;
mod node_15;
mod node_256;
mod node_3;
mod node_47;
mod simd;

pub(crate) use iter::KeyIter;
pub(crate) use iter::Lower;
pub(crate) use iter::NodeIter;
pub(crate) use iter::Upper;
pub(crate) use node_3::Node3;
pub(crate) use node_15::Node15;
pub(crate) use node_47::Node47;
pub(crate) use node_256::Node256;

use crate::raw::Edge;
use crate::raw::Smo;
use crate::raw::edge;
use crate::raw::edge::Meta as _;
use crate::raw::iter::Unbound;
use crate::stat;
use linear::Linear;

pub(crate) unsafe trait Node<M>: Default
where
    M: ribbit::Pack<Packed: edge::Meta>,
{
    const KIND: Kind;
    const LEN: usize;

    type Grow: Node<M>;
    type Shrink: Node<M>;

    fn len(&self) -> u8 {
        self.edges()
            .iter()
            .filter(|edge| !edge.load_packed(Ordering::Relaxed).is_null())
            .count() as u8
    }

    fn keys<L: iter::Lower, U: iter::Upper>(&self, lower: L, upper: U) -> KeyIter;

    fn entries<L: iter::Lower, U: iter::Upper>(&self, lower: L, upper: U) -> NodeIter<L, U, M> {
        unsafe { NodeIter::new(lower, upper, self.keys(lower, upper), self.edges()) }
    }

    fn edges(&self) -> &[Atomic<Edge<M>>];

    fn edges_mut(&mut self) -> &mut [Atomic<Edge<M>>];

    /// # Safety
    ///
    /// Implementer must guarantee that `Some(index)` is within `self.edges()`
    fn get_key(&self, key: u8) -> Option<u8>;

    #[inline]
    fn get(&self, key: u8) -> Option<&Atomic<Edge<M>>> {
        let index = self.get_key(key)? as usize;
        let edges = self.edges();
        Some(if cfg!(feature = "validate") {
            &edges[index]
        } else {
            unsafe { edges.get_unchecked(index) }
        })
    }

    /// # Safety
    ///
    /// Implementer must guarantee that `Some(index)` is within `self.edges()`
    fn get_or_insert_key(&self, key: u8) -> Option<u8>;

    #[inline]
    fn get_or_insert(&self, key: u8) -> Option<&Atomic<Edge<M>>> {
        let index = self.get_or_insert_key(key)? as usize;
        let edges = self.edges();
        Some(if cfg!(feature = "validate") {
            &edges[index]
        } else {
            unsafe { edges.get_unchecked(index) }
        })
    }

    /// # Safety
    ///
    /// Implementer must guarantee that `Some(index)` is within `self.edges()`
    fn insert_key(&mut self, key: u8) -> Option<u8>;

    #[inline]
    fn insert(&mut self, key: u8) -> Option<&mut Atomic<Edge<M>>> {
        let index = self.insert_key(key)? as usize;
        let edges = self.edges_mut();
        Some(if cfg!(feature = "validate") {
            &mut edges[index]
        } else {
            unsafe { edges.get_unchecked_mut(index) }
        })
    }

    fn freeze(&self) {
        let len = self.freeze_header();
        self.edges().iter().take(len).for_each(Edge::freeze)
    }

    fn freeze_header(&self) -> usize;

    fn replace<const LEN: usize, const FREEZE: bool>(
        &self,
        meta: ribbit::Packed<M>,
    ) -> (Smo, ribbit::Packed<Edge<M>>) {
        const {
            // HACK: can't use generic associated type as array length
            assert!(Self::LEN == LEN);
        }

        // Caller must not call replace if doomed to fail CAS
        validate!(!meta.is_frozen());

        // Can only call replace on nodes
        validate!(!meta.is_value());

        let mut keys = [0u8; LEN];
        let mut edges = [Edge::DEFAULT; LEN];

        if FREEZE {
            self.freeze();
        }

        let len = self
            .entries(Unbound::<()>::default(), Unbound::<()>::default())
            .map(|(key, edge)| (key, unsafe { edge.as_ref() }.load_packed(Ordering::Relaxed)))
            .filter(|(_, edge)| !edge.is_null())
            .map(|(key, edge)| match FREEZE {
                true => {
                    validate!(
                        edge.meta().is_frozen(),
                        "{} edge must be frozen before replace",
                        core::any::type_name::<Self>(),
                    );
                    (key, edge.unfreeze())
                }
                false => {
                    validate!(
                        !edge.meta().is_frozen(),
                        "{} edge must not be frozen",
                        core::any::type_name::<Self>(),
                    );
                    (key, edge)
                }
            })
            .zip(&mut keys)
            .zip(&mut edges)
            .map(|(((key_old, edge_old), key_new), edge_new)| {
                *key_new = key_old;
                *edge_new = edge_old;
            })
            .count();

        replace::<M, Self>(meta, &keys[..len], &edges[..len])
    }
}

fn replace<M: ribbit::Pack<Packed: edge::Meta>, N: Node<M>>(
    meta: ribbit::Packed<M>,
    keys: &[u8],
    edges: &[ribbit::Packed<Edge<M>>],
) -> (Smo, ribbit::Packed<Edge<M>>) {
    validate_eq!(keys.len(), edges.len());

    let len = keys.len();

    if len == 0 {
        return (Smo::DeleteNode, Edge::DEFAULT);
    } else if len == 1 {
        let key = keys[0];
        let edge = edges[0];
        if let Some(meta) = meta.compress(key, edge.meta()) {
            return (Smo::CompressEdge, edge.with_meta(meta));
        }
    }

    let keys = keys.iter().copied();
    let edges = edges.iter().copied();

    let edge = if len == N::LEN {
        unsafe { Edge::new_node_unchecked::<N::Grow, _, _>(meta, keys, edges) }
    } else if len < 4 {
        unsafe { Edge::new_node_unchecked::<Node3<_>, _, _>(meta, keys, edges) }
    } else if len < 16 {
        unsafe { Edge::new_node_unchecked::<Node15<_>, _, _>(meta, keys, edges) }
    } else if len < 48 {
        unsafe { Edge::new_node_unchecked::<Node47<_>, _, _>(meta, keys, edges) }
    } else {
        unsafe { Edge::new_node_unchecked::<Node256<_>, _, _>(meta, keys, edges) }
    };

    (Smo::ReplaceNode, edge)
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ribbit::Pack)]
#[ribbit(size = 2, eq, debug, packed(rename = "KindPacked"))]
pub(crate) enum Kind {
    Node3 = 0,
    Node15 = 1,
    Node47 = 2,
    Node256 = 3,
}

impl Default for Kind {
    fn default() -> Self {
        Self::Node3
    }
}

// We use a manual if-else chain instead of a match here because LLVM generates
// a jump table for the latter. In our experiments, we observe that a jump table
// in hot loops causes significant slowdowns: the jump table causes more branch
// mispredictions, and the mispredicted branches cause excess cache coherence
// traffic for cache lines that would otherwise be untouched.
//
// We use a macro instead of a function because there is no way to express mutually
// exclusive closures as parameters. We sometimes need $node3, $node15, $node47, and
// $node256 to borrow the same data mutably.
macro_rules! dispatch {
    ($kind:expr, $node3:expr, $node15:expr, $node47:expr, $node256:expr $(,)?) => {{
        if cfg!(feature = "opt-no-dispatch") {
            use crate::raw::node::Kind;
            use ribbit::Unpack as _;
            match $kind.unpack() {
                Kind::Node3 => $node3,
                Kind::Node15 => $node15,
                Kind::Node47 => $node47,
                Kind::Node256 => $node256,
            }
        } else {
            let kind = $kind.value.value();
            let hi = kind & 0b10;
            let lo = kind & 0b01;

            if hi == 0 {
                if lo == 0 { $node3 } else { $node15 }
            } else if lo == 0 {
                $node47
            } else {
                $node256
            }
        }
    }};
}
pub(super) use dispatch;

#[derive(ribbit::Pack)]
#[ribbit(size = 64, packed(rename = PtrPacked), eq, nonzero)]
pub(crate) struct Ptr<M> {
    #[ribbit(size = 2, get(vis = "pub(crate)"))]
    kind: Kind,

    #[ribbit(with(skip))]
    _placeholder: NonZeroU32,

    _meta: PhantomData<M>,
}

impl<M> Copy for Ptr<M> {}
impl<M> Clone for Ptr<M> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<M> Ptr<M> {
    const MASK_TAG: u64 = 0b111;
    const MASK_PTR: u64 = !Self::MASK_TAG;
}

impl<M> Ptr<M>
where
    M: ribbit::Pack<Packed: edge::Meta>,
{
    #[inline]
    pub(super) fn new<N: Node<M>>(node: Box<N>) -> ribbit::Packed<Self> {
        let ptr = NonNull::from(Box::leak(node));
        let kind = N::KIND as u64;

        validate_eq!(ptr.addr().get() as u64 & Self::MASK_TAG, 0);

        unsafe {
            ribbit::Packed::<Self>::new_unchecked(NonZeroU64::new_unchecked(
                kind | ptr.addr().get() as u64,
            ))
        }
    }

    #[inline]
    pub(super) fn new_ptr(ptr: NonNull<Node3<M>>) -> ribbit::Packed<Self> {
        unsafe {
            ribbit::Packed::<Self>::new_unchecked(NonZeroU64::new_unchecked(
                Node3::<M>::KIND as u64 | ptr.addr().get() as u64,
            ))
        }
    }

    #[inline]
    pub(crate) unsafe fn new_unchecked(raw: u64) -> ribbit::Packed<Self> {
        let node = unsafe { ribbit::Packed::<Option<Ptr<M>>>::new_unchecked(raw) };
        if cfg!(feature = "validate") {
            node.unwrap()
        } else {
            unsafe { node.unwrap_unchecked() }
        }
    }
}

impl<M> PtrPacked<M>
where
    M: ribbit::Pack<Packed: edge::Meta>,
{
    #[inline]
    pub(crate) fn raw(self) -> NonZeroU64 {
        self.value
    }

    #[inline]
    pub(crate) unsafe fn len(self) -> u8 {
        self.dispatch(
            |node| unsafe { node.as_ref() }.len(),
            |node| unsafe { node.as_ref() }.len(),
            |node| unsafe { node.as_ref() }.len(),
            |node| unsafe { node.as_ref() }.len(),
        )
    }

    #[inline]
    pub(crate) unsafe fn get<'g>(self, key: u8) -> Option<&'g Atomic<Edge<M>>> {
        self.dispatch(
            |node| unsafe { node.as_ref() }.get(key),
            |node| unsafe { node.as_ref() }.get(key),
            |node| unsafe { node.as_ref() }.get(key),
            |node| unsafe { node.as_ref() }.get(key),
        )
    }

    #[inline]
    pub(crate) unsafe fn get_or_insert<'g>(self, key: u8) -> Option<&'g Atomic<Edge<M>>> {
        self.dispatch(
            |node| unsafe { node.as_ref() }.get_or_insert(key),
            |node| unsafe { node.as_ref() }.get_or_insert(key),
            |node| unsafe { node.as_ref() }.get_or_insert(key),
            |node| unsafe { node.as_ref() }.get_or_insert(key),
        )
    }

    #[inline]
    pub(crate) unsafe fn replace<const FREEZE: bool>(
        self,
        parent: ribbit::Packed<M>,
    ) -> (Smo, ribbit::Packed<Edge<M>>) {
        self.dispatch(
            |node| unsafe { node.as_ref() }.replace::<3, FREEZE>(parent),
            |node| unsafe { node.as_ref() }.replace::<15, FREEZE>(parent),
            |node| unsafe { node.as_ref() }.replace::<47, FREEZE>(parent),
            |node| unsafe { node.as_ref() }.replace::<256, FREEZE>(parent),
        )
    }

    #[inline]
    pub(crate) unsafe fn entries<'g, L: Lower, U: Upper>(
        self,
        lower: L,
        upper: U,
    ) -> NodeIter<'g, L, U, M> {
        self.dispatch(
            |node| unsafe { node.as_ref() }.entries(lower, upper),
            |node| unsafe { node.as_ref() }.entries(lower, upper),
            |node| unsafe { node.as_ref() }.entries(lower, upper),
            |node| unsafe { node.as_ref() }.entries(lower, upper),
        )
    }

    /// # SAFETY
    ///
    /// Caller must ensure there are no other references to this node.
    pub(crate) unsafe fn deallocate(self, counter: stat::Counter) {
        stat::increment(counter);
        self.dispatch(
            |node| drop(unsafe { Box::from_raw(node.as_ptr()) }),
            |node| drop(unsafe { Box::from_raw(node.as_ptr()) }),
            |node| drop(unsafe { Box::from_raw(node.as_ptr()) }),
            |node| drop(unsafe { Box::from_raw(node.as_ptr()) }),
        )
    }

    /// # SAFETY
    ///
    /// Caller must ensure there are no other references to this node.
    pub(crate) unsafe fn deallocate_recursive(self, counter: stat::Counter) {
        stat::increment(counter);

        validate_eq!(self.kind(), Kind::Node3.pack());

        let ptr = self.value.get() & Ptr::<M>::MASK_PTR;
        let mut node = unsafe { Box::from_raw(Self::as_ptr::<Node3<M>>(ptr).as_ptr()) };
        unsafe {
            node.edges_mut()[0]
                .get_packed()
                .deallocate_recursive_unchecked(counter);
        }

        drop(node);
    }

    #[inline(always)]
    pub(crate) fn dispatch<N3, N15, N47, N256, T>(
        self,
        node_3: N3,
        node_15: N15,
        node_47: N47,
        node_256: N256,
    ) -> T
    where
        N3: FnOnce(NonNull<Node3<M>>) -> T,
        N15: FnOnce(NonNull<Node15<M>>) -> T,
        N47: FnOnce(NonNull<Node47<M>>) -> T,
        N256: FnOnce(NonNull<Node256<M>>) -> T,
    {
        let ptr = self.value.get() & Ptr::<M>::MASK_PTR;
        dispatch!(
            self.kind(),
            node_3(unsafe { Self::as_ptr(ptr) }),
            node_15(unsafe { Self::as_ptr(ptr) }),
            node_47(unsafe { Self::as_ptr(ptr) }),
            node_256(unsafe { Self::as_ptr(ptr) }),
        )
    }

    #[inline(always)]
    unsafe fn as_ptr<N>(ptr: u64) -> NonNull<N>
    where
        N: Node<M>,
    {
        let node = NonNull::new(ptr as *mut N);
        if cfg!(feature = "validate") {
            node.unwrap()
        } else {
            unsafe { node.unwrap_unchecked() }
        }
    }
}

impl<M> Debug for PtrPacked<M> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Node")
            .field("kind", &self.kind())
            .field("ptr", &(self.value.get() & Ptr::<M>::MASK_PTR))
            .finish()
    }
}
