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

    fn freeze(&self);

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
            return (Smo::Destroy, Edge::DEFAULT);
        } else if len == 1 {
            let key = keys[0];
            let edge = edges[0];
            if let Some(meta) = meta.compress(key, edge.meta()) {
                return (Smo::Compress, edge.with_meta(meta));
            }
        }

        let keys = keys.into_iter().take(len);
        let edges = edges.into_iter().take(len);

        if len == Self::LEN {
            (Smo::Grow, unsafe {
                Edge::new_node_unchecked::<Self::Grow, _, _>(meta, keys, edges)
            })
        } else {
            // Catch-all:
            (Smo::Replace, unsafe {
                Edge::new_node_unchecked::<Self, _, _>(meta, keys, edges)
            })
        }
    }
}

#[derive(ribbit::Pack)]
#[ribbit(size = 64, packed(rename = PtrPacked), eq, nonzero)]
pub(crate) struct Ptr<C> {
    #[ribbit(size = 2, get(vis = "pub(crate)"))]
    kind: Kind,

    pub(crate) scan: bool,

    #[ribbit(with(skip))]
    _placeholder: NonZeroU32,

    _compressed: PhantomData<C>,
}

impl<C> Copy for Ptr<C> {}
impl<C> Clone for Ptr<C> {
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

    pub(super) fn new_ptr(ptr: NonNull<Node3<M>>) -> ribbit::Packed<Self> {
        unsafe {
            ribbit::Packed::<Self>::new_unchecked(NonZeroU64::new_unchecked(
                Node3::<M>::KIND as u64 | ptr.addr().get() as u64,
            ))
        }
    }

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
    pub(super) fn raw(self) -> NonZeroU64 {
        self.value
    }

    #[inline(never)]
    pub(crate) unsafe fn get_unchecked<'g>(self, key: u8) -> Option<&'g Atomic<Edge<M>>> {
        let ptr = self.value.get() & Ptr::<M>::MASK_PTR;
        let kind = self.kind();

        // if kind == node::Kind::NODE_3 {
        //     unsafe { as_ref::<_, Node3<M>>(ptr) }.get(key)
        // } else if kind == node::Kind::NODE_15 {
        //     unsafe { as_ref::<_, Node15<M>>(ptr) }.get(key)
        // } else if kind == node::Kind::NODE_47 {
        //     unsafe { as_ref::<_, Node47<M>>(ptr) }.get(key)
        // } else {
        //     validate_eq!(kind, node::Kind::NODE_256);
        //     unsafe { as_ref::<_, Node256<M>>(ptr) }.get(key)
        // }

        let hi = kind.raw() >> 1;
        let lo = kind.raw() & 0b1;

        if hi == 0 {
            if lo == 0 {
                unsafe { as_ref::<_, Node3<M>>(ptr) }.get(key)
            } else {
                unsafe { as_ref::<_, Node15<M>>(ptr) }.get(key)
            }
        } else if lo == 0 {
            unsafe { as_ref::<_, Node47<M>>(ptr) }.get(key)
        } else {
            validate_eq!(kind, Kind::NODE_256);
            unsafe { as_ref::<_, Node256<M>>(ptr) }.get(key)
        }
    }

    #[inline]
    pub(crate) unsafe fn get_or_insert_unchecked<'g>(self, key: u8) -> Option<&'g Atomic<Edge<M>>> {
        let ptr = self.value.get() & Ptr::<M>::MASK_PTR;
        let kind = self.kind();

        // if kind == node::Kind::NODE_3 {
        //     unsafe { as_ref::<_, Node3<M>>(ptr) }.get_or_insert(key)
        // } else if kind == node::Kind::NODE_15 {
        //     unsafe { as_ref::<_, Node15<M>>(ptr) }.get_or_insert(key)
        // } else if kind == node::Kind::NODE_47 {
        //     unsafe { as_ref::<_, Node47<M>>(ptr) }.get_or_insert(key)
        // } else {
        //     validate_eq!(kind, node::Kind::NODE_256);
        //     unsafe { as_ref::<_, Node256<M>>(ptr) }.get_or_insert(key)
        // }

        let hi = kind.raw() >> 1;
        let lo = kind.raw() & 0b1;

        if hi == 0 {
            if lo == 0 {
                unsafe { as_ref::<_, Node3<M>>(ptr) }.get_or_insert(key)
            } else {
                unsafe { as_ref::<_, Node15<M>>(ptr) }.get_or_insert(key)
            }
        } else if lo == 0 {
            unsafe { as_ref::<_, Node47<M>>(ptr) }.get_or_insert(key)
        } else {
            validate_eq!(kind, Kind::NODE_256);
            unsafe { as_ref::<_, Node256<M>>(ptr) }.get_or_insert(key)
        }
    }

    #[inline]
    pub(crate) unsafe fn replace_unchecked(
        self,
        parent: ribbit::Packed<M>,
    ) -> (Smo, ribbit::Packed<Edge<M>>) {
        let ptr = self.value.get() & Ptr::<M>::MASK_PTR;
        let kind = self.kind();

        // if kind == node::Kind::NODE_3 {
        //     unsafe { as_ref::<_, Node3<M>>(ptr) }.replace::<3>(parent)
        // } else if kind == node::Kind::NODE_15 {
        //     unsafe { as_ref::<_, Node15<M>>(ptr) }.replace::<15>(parent)
        // } else if kind == node::Kind::NODE_47 {
        //     unsafe { as_ref::<_, Node47<M>>(ptr) }.replace::<47>(parent)
        // } else {
        //     validate_eq!(kind, node::Kind::NODE_256);
        //     unsafe { as_ref::<_, Node256<M>>(ptr) }.replace::<256>(parent)
        // }

        let hi = kind.raw() >> 1;
        let lo = kind.raw() & 0b1;

        if hi == 0 {
            if lo == 0 {
                unsafe { as_ref::<_, Node3<M>>(ptr) }.replace::<3>(parent)
            } else {
                unsafe { as_ref::<_, Node15<M>>(ptr) }.replace::<15>(parent)
            }
        } else if lo == 0 {
            unsafe { as_ref::<_, Node47<M>>(ptr) }.replace::<47>(parent)
        } else {
            validate_eq!(kind, Kind::NODE_256);
            unsafe { as_ref::<_, Node256<M>>(ptr) }.replace::<256>(parent)
        }
    }

    #[inline]
    pub(crate) unsafe fn entries_unchecked<'g, L: Lower, U: Upper>(
        self,
        lower: L,
        upper: U,
    ) -> NodeIter<'g, L, U, M> {
        let ptr = self.value.get() & Ptr::<M>::MASK_PTR;
        let kind = self.kind();

        // if kind == node::Kind::NODE_3 {
        //     unsafe { as_ref::<_, Node3<M>>(ptr) }.entries(lower, upper)
        // } else if kind == node::Kind::NODE_15 {
        //     unsafe { as_ref::<_, Node15<M>>(ptr) }.entries(lower, upper)
        // } else if kind == node::Kind::NODE_47 {
        //     unsafe { as_ref::<_, Node47<M>>(ptr) }.entries(lower, upper)
        // } else {
        //     validate_eq!(kind, node::Kind::NODE_256);
        //     unsafe { as_ref::<_, Node256<M>>(ptr) }.entries(lower, upper)
        // }

        let hi = kind.raw() >> 1;
        let lo = kind.raw() & 0b1;

        if hi == 0 {
            if lo == 0 {
                unsafe { as_ref::<_, Node3<M>>(ptr) }.entries(lower, upper)
            } else {
                unsafe { as_ref::<_, Node15<M>>(ptr) }.entries(lower, upper)
            }
        } else if lo == 0 {
            unsafe { as_ref::<_, Node47<M>>(ptr) }.entries(lower, upper)
        } else {
            validate_eq!(kind, Kind::NODE_256);
            unsafe { as_ref::<_, Node256<M>>(ptr) }.entries(lower, upper)
        }
    }

    /// # SAFETY
    ///
    /// Caller must ensure there are no other references to this node.
    #[inline]
    pub(crate) unsafe fn deallocate_unchecked(self, counter: stat::Counter) {
        stat::increment(counter);

        let ptr = self.value.get() & Ptr::<M>::MASK_PTR;
        let kind = self.kind();

        // if kind == node::Kind::NODE_3 {
        //     drop(Box::from_raw(ptr as *mut Node3<M>))
        // } else if kind == node::Kind::NODE_15 {
        //     drop(Box::from_raw(ptr as *mut Node15<M>))
        // } else if kind == node::Kind::NODE_47 {
        //     drop(Box::from_raw(ptr as *mut Node47<M>))
        // } else {
        //     validate_eq!(kind, node::Kind::NODE_256);
        //     drop(Box::from_raw(ptr as *mut Node256<M>))
        // }

        let hi = kind.raw() >> 1;
        let lo = kind.raw() & 0b1;

        if hi == 0 {
            if lo == 0 {
                drop(Box::from_raw(ptr as *mut Node3<M>))
            } else {
                drop(Box::from_raw(ptr as *mut Node15<M>))
            }
        } else if lo == 0 {
            drop(Box::from_raw(ptr as *mut Node47<M>))
        } else {
            validate_eq!(kind, Kind::NODE_256);
            drop(Box::from_raw(ptr as *mut Node256<M>))
        }
    }

    /// # SAFETY
    ///
    /// Caller must ensure there are no other references to this node.
    pub(crate) unsafe fn deallocate_recursive_unchecked(self, counter: stat::Counter) {
        stat::increment(counter);

        let ptr = self.value.get() & Ptr::<M>::MASK_PTR;
        let kind = self.kind();

        // if kind == node::Kind::NODE_3 {
        //     drop(Box::from_raw(ptr as *mut Node3<M>))
        // } else if kind == node::Kind::NODE_15 {
        //     drop(Box::from_raw(ptr as *mut Node15<M>))
        // } else if kind == node::Kind::NODE_47 {
        //     drop(Box::from_raw(ptr as *mut Node47<M>))
        // } else {
        //     validate_eq!(kind, node::Kind::NODE_256);
        //     drop(Box::from_raw(ptr as *mut Node256<M>))
        // }

        let hi = kind.raw() >> 1;
        let lo = kind.raw() & 0b1;

        unsafe {
            if hi == 0 {
                if lo == 0 {
                    let mut node = Box::from_raw(ptr as *mut Node3<M>);
                    if let Some(child) = node.edges_mut()[0].get_packed().as_node() {
                        child.deallocate_recursive_unchecked(counter);
                    }
                    drop(node);
                } else {
                    let mut node = Box::from_raw(ptr as *mut Node15<M>);
                    if let Some(child) = node.edges_mut()[0].get_packed().as_node() {
                        child.deallocate_recursive_unchecked(counter);
                    }
                    drop(node);
                }
            } else if lo == 0 {
                let mut node = Box::from_raw(ptr as *mut Node47<M>);
                if let Some(child) = node.edges_mut()[0].get_packed().as_node() {
                    child.deallocate_recursive_unchecked(counter);
                }
                drop(node);
            } else {
                validate_eq!(kind, Kind::NODE_256);
                let mut node = Box::from_raw(ptr as *mut Node256<M>);
                if let Some(child) = node.edges_mut()[0].get_packed().as_node() {
                    child.deallocate_recursive_unchecked(counter);
                }
                drop(node);
            }
        }
    }
}

#[inline]
unsafe fn as_ref<'g, M, N>(ptr: u64) -> &'g N
where
    M: ribbit::Pack<Packed: edge::Meta>,
    N: Node<M> + 'g,
{
    let node = unsafe { (ptr as *const N).as_ref() };
    validate!(node.is_some());
    unsafe { node.unwrap_unchecked() }
}

impl<M> Debug for PtrPacked<M> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Node")
            .field("kind", &self.kind())
            .field("scan", &self.scan())
            .field("ptr", &(self.value.get() & Ptr::<M>::MASK_PTR))
            .finish()
    }
}

/// Node-related structural modification operation. Requires freezing.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Smo {
    /// Node shrink (smaller size)
    #[expect(dead_code)]
    Shrink,

    /// Node replacement (same size)
    Replace,

    /// Node growth (larger size)
    Grow,

    /// Node elimination
    Destroy,

    /// Path compression
    Compress,
}

impl Smo {
    /// Whether this operation allocates a new node.
    #[inline]
    pub(crate) fn is_allocate(self) -> bool {
        match self {
            Self::Destroy | Self::Compress => false,
            Self::Grow | Self::Replace | Self::Shrink => true,
        }
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
