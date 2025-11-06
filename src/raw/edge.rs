use core::fmt::Debug;
use core::marker::PhantomData;
use core::num::NonZeroU32;
use core::num::NonZeroU64;
use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;
use ribbit::u56;
use ribbit::u6;
use ribbit::OptionExt as _;

use crate::byte;
use crate::raw::node;
use crate::raw::node::Node15;
use crate::raw::node::Node256;
use crate::raw::node::Node3;
use crate::stat;

#[derive(ribbit::Pack)]
#[ribbit(size = 128, packed(rename = EdgePacked))]
pub struct Edge<C> {
    #[ribbit(size = 64)]
    pub(crate) meta: Meta,

    data: u64,

    // FIXME: swap out edge type
    _compressed: PhantomData<C>,
}

impl<C> Copy for Edge<C> {}
impl<C> Clone for Edge<C> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<C> Default for EdgePacked<C> {
    fn default() -> Self {
        Edge::DEFAULT
    }
}

impl<C> Edge<C> {
    pub(crate) const DEFAULT: ribbit::Packed<Self> = ribbit::Packed::<Self>::new(Meta::DEFAULT, 0);

    #[inline]
    pub(crate) fn freeze(edge: &Atomic128<Self>) {
        let mut old = edge.load_packed(Ordering::Relaxed);

        while !old.meta().is_frozen() {
            match edge.compare_exchange_packed(
                old,
                old.with_meta(old.meta().with_frozen(true)),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(conflict) => old = conflict,
            }
        }
    }

    #[cold]
    pub(crate) fn new_node<N, I>(key: byte::Array, edges: I) -> ribbit::Packed<Self>
    where
        N: node::Node<C>,
        I: IntoIterator<Item = (u8, ribbit::Packed<Edge<C>>)>,
    {
        let mut node = Box::new(N::default());

        for (key, edge) in edges {
            node.reserve(key)
                .expect("Node can fit all edges")
                .set_packed(edge);
        }

        ribbit::Packed::<Self>::new(Meta::DEFAULT.with_key(key), Node::new(node).value.get())
    }

    pub(crate) fn new_value(key: byte::Array, value: u64) -> ribbit::Packed<Self> {
        ribbit::Packed::<Self>::new(Meta::VALUE.with_key(key), value)
    }
}

impl<C> EdgePacked<C> {
    // FIXME: remove
    #[inline]
    pub(crate) fn erase(self) -> ribbit::Packed<Edge<()>> {
        ribbit::Packed::<Edge<()>>::new(self.meta(), self.data())
    }

    #[inline]
    pub(crate) fn is_null(self) -> bool {
        !self.meta().is_value() && self.data() == 0
    }

    #[inline]
    pub(crate) fn as_node(self) -> Option<ribbit::Packed<Node<C>>> {
        if self.meta().is_value() {
            return None;
        }

        unsafe { ribbit::Packed::<Option<Node<C>>>::new_unchecked(self.data()) }
    }

    #[inline]
    pub(crate) fn as_value(self) -> Option<u64> {
        self.meta().is_value().then(|| self.data())
    }

    #[inline]
    pub(crate) fn child(self) -> Option<Child<C>> {
        let data = self.data();
        if self.meta().is_value() {
            Some(Child::Value(data))
        } else {
            unsafe { ribbit::Packed::<Option<Node<C>>>::new_unchecked(data) }.map(Child::Node)
        }
    }

    #[inline]
    pub(crate) fn with_node(self, node: ribbit::Packed<Node<C>>) -> Self {
        self.with_data(node.value.get())
    }

    #[inline]
    pub(crate) fn with_value(self, value: u64) -> Self {
        self.with_data(value)
    }

    #[inline]
    pub(crate) fn unfreeze(self) -> Self {
        self.with_meta(self.meta().with_frozen(false))
    }

    #[inline]
    pub(crate) unsafe fn deallocate<F>(self, deallocate_value: F, counter: stat::Counter)
    where
        F: FnOnce(u64),
    {
        if self.is_null() {
            return;
        }

        unsafe { self.deallocate_unchecked(deallocate_value, counter) }
    }

    #[inline]
    pub(crate) unsafe fn deallocate_unchecked<F>(self, deallocate_value: F, counter: stat::Counter)
    where
        F: FnOnce(u64),
    {
        match self.child() {
            None if cfg!(feature = "validate") => unreachable!(),
            None => unsafe { core::hint::unreachable_unchecked() },
            Some(Child::Node(node)) => node.deallocate_unchecked(counter),
            Some(Child::Value(value)) => deallocate_value(value),
        }
    }
}

impl<C> Debug for EdgePacked<C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut debug = f.debug_struct("Edge");

        debug.field("meta", &self.meta());
        debug.field("data", &self.child());

        debug.finish()
    }
}

#[derive(Copy, Clone, Default, PartialEq, Eq, ribbit::Pack)]
#[ribbit(size = 64, packed(rename = MetaPacked), eq)]
pub(crate) struct Meta {
    #[ribbit(with(skip))]
    _placeholder_len: u6,
    value: bool,
    frozen: bool,
    #[ribbit(with(skip))]
    _placeholder_array: u56,
}

impl Meta {
    pub(crate) const DEFAULT: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(u6::new(0), false, false, u56::new(0));

    pub(crate) const VALUE: ribbit::Packed<Self> = Self::DEFAULT.with_value(true);
}

impl MetaPacked {
    #[inline]
    pub(crate) fn key(self) -> byte::Array {
        byte::Array::new_masked(self.value)
    }

    #[inline]
    pub(crate) fn with_key(self, key: byte::Array) -> Self {
        unsafe { Self::new_unchecked(self.value & !byte::Array::MASK | key.value()) }
    }

    #[inline]
    pub(crate) fn is_value(self) -> bool {
        self.value()
    }

    #[inline]
    pub(crate) fn is_frozen(self) -> bool {
        self.frozen()
    }
}

impl Debug for MetaPacked {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Meta")
            .field("value", &self.is_value())
            .field("frozen", &self.is_frozen())
            .field("key", &self.key())
            .finish()
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    /// Node creation
    Create,

    /// Path expansion
    Expand,

    /// Value insertion
    Insert,

    /// Value removal
    #[expect(dead_code)]
    Remove,
}

impl Op {
    /// Whether this operation allocates a new node.
    #[inline]
    pub(crate) fn is_allocate(self) -> bool {
        match self {
            Self::Insert | Self::Remove => false,
            Self::Create | Self::Expand => true,
        }
    }
}

pub(crate) enum Child<C> {
    Node(ribbit::Packed<Node<C>>),
    Value(u64),
}

impl<C> Debug for Child<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Node(node) => f.debug_tuple("Node").field(node).finish(),
            Self::Value(value) => f.debug_tuple("Value").field(value).finish(),
        }
    }
}

#[derive(ribbit::Pack)]
#[ribbit(size = 64, packed(rename = NodePacked), eq, nonzero)]
pub struct Node<C> {
    #[ribbit(size = 2)]
    kind: node::Kind,

    pub(crate) scan: bool,

    #[ribbit(with(skip))]
    _placeholder: NonZeroU32,

    _compressed: PhantomData<C>,
}

impl<C> Copy for Node<C> {}
impl<C> Clone for Node<C> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<C> Node<C> {
    const MASK_TAG: u64 = 0b111;
    const MASK_PTR: u64 = !Self::MASK_TAG;

    #[inline]
    fn new<N: node::Node<C>>(node: Box<N>) -> ribbit::Packed<Self> {
        let ptr = NonNull::from(Box::leak(node));
        let kind = N::KIND as u64;

        validate_eq!(ptr.addr().get() as u64 & Self::MASK_TAG, 0);

        unsafe {
            ribbit::Packed::<Self>::new_unchecked(NonZeroU64::new_unchecked(
                kind | ptr.addr().get() as u64,
            ))
        }
    }
}

impl<C> NodePacked<C> {
    #[inline]
    pub(crate) fn is_ref(self, node: node::Ref<'_, C>) -> bool {
        let ptr = match node {
            node::Ref::Node3(node) => node as *const _ as u64,
            node::Ref::Node15(node) => node as *const _ as u64,
            node::Ref::Node256(node) => node as *const _ as u64,
        };

        self.value.get() & Node::<C>::MASK_PTR == ptr
    }

    #[inline]
    pub(crate) unsafe fn into_ref_unchecked<'g>(self) -> node::Ref<'g, C> {
        #[inline]
        unsafe fn as_ref<'g, C, N: node::Node<C> + 'g>(ptr: u64) -> &'g N {
            let node = unsafe { (ptr as *const N).as_ref() };
            validate!(node.is_some());
            unsafe { node.unwrap_unchecked() }
        }

        let ptr = self.value.get() & Node::<C>::MASK_PTR;
        let kind = self.kind();

        if kind == node::Kind::NODE_3 {
            node::Ref::Node3(unsafe { as_ref::<_, Node3<C>>(ptr) })
        } else if kind == node::Kind::NODE_15 {
            node::Ref::Node15(unsafe { as_ref::<_, Node15<C>>(ptr) })
        } else {
            validate_eq!(kind, node::Kind::NODE_256);
            node::Ref::Node256(unsafe { as_ref::<_, Node256<C>>(ptr) })
        }
    }

    /// # SAFETY
    ///
    /// Caller must ensure there are no other references to this node.
    #[inline]
    pub(crate) unsafe fn deallocate_unchecked(self, counter: stat::Counter) {
        stat::increment(counter);

        let ptr = self.value.get() & Node::<C>::MASK_PTR;
        let kind = self.kind();

        if kind == node::Kind::NODE_3 {
            drop(Box::from_raw(ptr as *mut Node3<C>))
        } else if kind == node::Kind::NODE_15 {
            drop(Box::from_raw(ptr as *mut Node15<C>))
        } else {
            validate_eq!(kind, node::Kind::NODE_256);
            drop(Box::from_raw(ptr as *mut Node256<C>))
        }
    }
}

impl<C> Debug for NodePacked<C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Node")
            .field("kind", &self.kind())
            .field("scan", &self.scan())
            .field("ptr", &(self.value.get() & Node::<C>::MASK_PTR))
            .finish()
    }
}
