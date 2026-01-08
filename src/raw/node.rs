use core::fmt::Debug;
use core::marker::PhantomData;
use core::num::NonZeroU32;
use core::num::NonZeroU64;
use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use ribbit::Atomic;
use ribbit::OptionExt as _;

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
pub(crate) use node_15::Node15;
pub(crate) use node_256::Node256;
pub(crate) use node_3::Node3;
pub(crate) use node_47::Node47;

use crate::raw::edge;
use crate::raw::edge::Meta as _;
use crate::raw::iter::Unbound;
use crate::raw::Edge;
use crate::raw::Smo;
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

    fn replace<const LEN_: usize>(
        &self,
        meta: ribbit::Packed<M>,
    ) -> (Smo, ribbit::Packed<Edge<M>>) {
        const {
            assert!(Self::LEN == LEN_);
        }

        // Caller must not call replace if doomed to fail CAS
        validate!(!meta.is_frozen());

        // Can only call replace on nodes
        validate!(!meta.is_value());

        let mut keys = [0u8; LEN_];
        let mut edges = [Edge::DEFAULT; LEN_];

        self.freeze();

        let len = self
            .entries(Unbound, Unbound)
            .map(|(key, edge)| (key, edge.load_packed(Ordering::Relaxed)))
            .filter(|(_, edge)| !edge.is_null())
            .map(|(key, edge)| {
                validate!(
                    edge.meta().is_frozen(),
                    "{} edge must be frozen before replace",
                    core::any::type_name::<Self>(),
                );
                (key, edge.unfreeze())
            })
            .zip(&mut keys)
            .zip(&mut edges)
            .map(|(((key_old, edge_old), key_new), edge_new)| {
                *key_new = key_old;
                *edge_new = edge_old;
            })
            .count();

        if len == 0 {
            return (Smo::DeleteNode, Edge::DEFAULT);
        } else if len == 1 {
            let key = keys[0];
            let edge = edges[0];
            if let Some(meta) = meta.compress(key, edge.meta()) {
                return (Smo::CompressEdge, edge.with_meta(meta));
            }
        }

        let keys = keys.into_iter().take(len);
        let edges = edges.into_iter().take(len);

        if len == Self::LEN {
            (Smo::ExpandNode, unsafe {
                Edge::new_node_unchecked::<Self::Grow, _, _>(meta, keys, edges)
            })
        } else {
            // Catch-all:
            (Smo::ReplaceNode, unsafe {
                Edge::new_node_unchecked::<Self, _, _>(meta, keys, edges)
            })
        }
    }
}

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
    pub(crate) fn new_unchecked(raw: u64) -> ribbit::Packed<Self> {
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
    pub(super) fn raw(self) -> NonZeroU64 {
        self.value
    }

    #[inline]
    pub(crate) unsafe fn get_unchecked<'g>(self, key: u8) -> Option<&'g Atomic<Edge<M>>> {
        self.dispatch(
            |node| node.get(key),
            |node| node.get(key),
            |node| node.get(key),
            |node| node.get(key),
        )
    }

    #[inline]
    pub(crate) unsafe fn get_or_insert_unchecked<'g>(self, key: u8) -> Option<&'g Atomic<Edge<M>>> {
        self.dispatch(
            |node| node.get_or_insert(key),
            |node| node.get_or_insert(key),
            |node| node.get_or_insert(key),
            |node| node.get_or_insert(key),
        )
    }

    #[inline]
    pub(crate) unsafe fn replace_unchecked(
        self,
        parent: ribbit::Packed<M>,
    ) -> (Smo, ribbit::Packed<Edge<M>>) {
        self.dispatch(
            |node| node.replace::<3>(parent),
            |node| node.replace::<15>(parent),
            |node| node.replace::<47>(parent),
            |node| node.replace::<256>(parent),
        )
    }

    #[inline]
    pub(crate) unsafe fn entries_unchecked<'g, L: Lower, U: Upper>(
        self,
        lower: L,
        upper: U,
    ) -> NodeIter<'g, L, U, M> {
        self.dispatch(
            |node| node.entries(lower, upper),
            |node| node.entries(lower, upper),
            |node| node.entries(lower, upper),
            |node| node.entries(lower, upper),
        )
    }

    /// # SAFETY
    ///
    /// Caller must ensure there are no other references to this node.
    pub(crate) unsafe fn deallocate_unchecked(self, counter: stat::Counter) {
        stat::increment(counter);
        self.dispatch(
            |node| drop(Box::from_raw((node as *const Node3<_>).cast_mut())),
            |node| drop(Box::from_raw((node as *const Node15<_>).cast_mut())),
            |node| drop(Box::from_raw((node as *const Node47<_>).cast_mut())),
            |node| drop(Box::from_raw((node as *const Node256<_>).cast_mut())),
        )
    }

    /// # SAFETY
    ///
    /// Caller must ensure there are no other references to this node.
    pub(crate) unsafe fn deallocate_recursive_unchecked(self, counter: stat::Counter) {
        stat::increment(counter);

        self.dispatch(
            |node| {
                let mut node = Box::from_raw((node as *const Node3<_>).cast_mut());
                if let Some(child) = node.edges_mut()[0].get_packed().as_node() {
                    child.deallocate_recursive_unchecked(counter);
                }
                drop(node);
            },
            |node| {
                let mut node = Box::from_raw((node as *const Node15<_>).cast_mut());
                if let Some(child) = node.edges_mut()[0].get_packed().as_node() {
                    child.deallocate_recursive_unchecked(counter);
                }
                drop(node);
            },
            |node| {
                let mut node = Box::from_raw((node as *const Node47<_>).cast_mut());
                if let Some(child) = node.edges_mut()[0].get_packed().as_node() {
                    child.deallocate_recursive_unchecked(counter);
                }
                drop(node);
            },
            |node| {
                let mut node = Box::from_raw((node as *const Node256<_>).cast_mut());
                if let Some(child) = node.edges_mut()[0].get_packed().as_node() {
                    child.deallocate_recursive_unchecked(counter);
                }
                drop(node);
            },
        );
    }

    #[inline(always)]
    fn dispatch<'g, N3, N15, N47, N256, T>(
        self,
        node_3: N3,
        node_15: N15,
        node_47: N47,
        node_256: N256,
    ) -> T
    where
        N3: FnOnce(&'g Node3<M>) -> T,
        N15: FnOnce(&'g Node15<M>) -> T,
        N47: FnOnce(&'g Node47<M>) -> T,
        N256: FnOnce(&'g Node256<M>) -> T,
        M: 'g,
    {
        let ptr = self.value.get() & Ptr::<M>::MASK_PTR;
        let kind = self.kind().raw();
        let hi = kind & 0b10;
        let lo = kind & 0b01;

        if hi == 0 {
            if lo == 0 {
                node_3(unsafe { Self::as_ref(ptr) })
            } else {
                node_15(unsafe { Self::as_ref(ptr) })
            }
        } else if lo == 0 {
            node_47(unsafe { Self::as_ref(ptr) })
        } else {
            node_256(unsafe { Self::as_ref(ptr) })
        }
    }

    #[inline(always)]
    unsafe fn as_ref<'g, N>(ptr: u64) -> &'g N
    where
        N: Node<M> + 'g,
    {
        let node = unsafe { (ptr as *const N).as_ref() };
        validate!(node.is_some());
        unsafe { node.unwrap_unchecked() }
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

impl Kind {
    pub(crate) const NODE_3: ribbit::Packed<Kind> = ribbit::Packed::<Kind>::new_node3();
    pub(crate) const NODE_15: ribbit::Packed<Kind> = ribbit::Packed::<Kind>::new_node15();
    pub(crate) const NODE_47: ribbit::Packed<Kind> = ribbit::Packed::<Kind>::new_node47();
    pub(crate) const NODE_256: ribbit::Packed<Kind> = ribbit::Packed::<Kind>::new_node256();
}

impl KindPacked {
    pub(crate) fn raw(self) -> u8 {
        self.value.value()
    }
}
