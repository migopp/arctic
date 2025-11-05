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
use crate::value;

#[derive(ribbit::Pack)]
#[ribbit(size = 128, packed(rename = EdgePacked))]
pub struct Edge<V> {
    #[ribbit(size = 0)]
    _value: PhantomData<V>,

    #[ribbit(size = 64)]
    pub(crate) meta: Meta,

    data: u64,
}

impl<V> Copy for Edge<V> {}
impl<V> Clone for Edge<V> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<V> Default for Edge<V> {
    fn default() -> Self {
        Self {
            _value: PhantomData,
            meta: Meta::default(),
            data: 0,
        }
    }
}

impl<V> Edge<V> {
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
        N: node::Info<V>,
        I: IntoIterator<Item = (u8, ribbit::Packed<Edge<V>>)>,
    {
        let mut node = Box::new(N::default());

        for (key, edge) in edges {
            node.reserve(key)
                .expect("Node can fit all edges")
                .set_packed(edge);
        }

        ribbit::Packed::<Self>::new(Meta::DEFAULT.with_key(key), Node::new(node).value.get())
    }

    pub(crate) fn new_value(
        key: byte::Array,
        value: ribbit::Packed<Value<V>>,
    ) -> ribbit::Packed<Self> {
        ribbit::Packed::<Self>::new(Meta::VALUE.with_key(key), value.value)
    }
}

impl<V> EdgePacked<V> {
    #[inline]
    pub(crate) fn is_null(self) -> bool {
        !self.meta().is_value() && self.data() == 0
    }

    #[inline]
    pub(crate) fn as_node(self) -> Option<ribbit::Packed<Node<V>>> {
        if self.meta().is_value() {
            return None;
        }

        unsafe { ribbit::Packed::<Option<Node<V>>>::new_unchecked(self.data()) }
    }

    #[inline]
    pub(crate) fn as_value(self) -> Option<ribbit::Packed<Value<V>>> {
        self.meta()
            .is_value()
            .then(|| unsafe { ribbit::Packed::<Value<V>>::new_unchecked(self.data()) })
    }

    #[inline]
    pub(crate) fn child(self) -> Option<Child<V>> {
        let data = self.data();
        if self.meta().is_value() {
            Some(Child::Value(unsafe {
                ribbit::Packed::<Value<V>>::new_unchecked(data)
            }))
        } else {
            unsafe { ribbit::Packed::<Option<Node<V>>>::new_unchecked(self.data()) }
                .map(Child::Node)
        }
    }

    #[inline]
    pub(crate) fn with_node(self, node: ribbit::Packed<Node<V>>) -> Self {
        self.with_data(node.value.get())
    }

    #[inline]
    pub(crate) fn with_value(self, value: ribbit::Packed<Value<V>>) -> Self {
        self.with_data(value.value)
    }

    #[inline]
    pub(crate) fn unfreeze(self) -> Self {
        self.with_meta(self.meta().with_frozen(false))
    }
}

impl<V: value::Value> EdgePacked<V> {
    /// # SAFETY
    ///
    /// Caller must ensure there are no references to the child of this edge.
    #[inline]
    pub(crate) unsafe fn deallocate(self, counter: stat::Counter) {
        match self.child() {
            None => (),
            Some(Child::Value(value)) => value.deallocate_unchecked(counter),
            Some(Child::Node(node)) => node.deallocate_unchecked(counter),
        }
    }

    /// # SAFETY
    ///
    /// Caller must ensure there are no references to the child of this edge,
    /// and that the child is non-null.
    #[inline]
    pub(crate) unsafe fn deallocate_unchecked(self, counter: stat::Counter) {
        match if cfg!(feature = "validate") {
            self.child().unwrap()
        } else {
            unsafe { self.child().unwrap_unchecked() }
        } {
            Child::Value(value) => value.deallocate_unchecked(counter),
            Child::Node(node) => node.deallocate_unchecked(counter),
        }
    }
}

impl<V> Debug for EdgePacked<V> {
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

pub(crate) enum Child<V> {
    Node(ribbit::Packed<Node<V>>),
    Value(ribbit::Packed<Value<V>>),
}

impl<V> Debug for Child<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Node(node) => f.debug_tuple("Node").field(node).finish(),
            Self::Value(value) => f.debug_tuple("Value").field(value).finish(),
        }
    }
}

#[derive(ribbit::Pack)]
#[ribbit(size = 64, packed(rename = NodePacked), eq, nonzero)]
pub struct Node<V> {
    #[ribbit(size = 0)]
    _value: PhantomData<V>,

    #[ribbit(size = 2)]
    kind: node::Kind,

    pub(crate) scan: bool,

    #[ribbit(with(skip))]
    _placeholder: NonZeroU32,
}

impl<V> Copy for Node<V> {}
impl<V> Clone for Node<V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<V> Node<V> {
    const MASK_TAG: u64 = 0b111;
    const MASK_PTR: u64 = !Self::MASK_TAG;

    #[inline]
    fn new<N: node::Info<V>>(node: Box<N>) -> ribbit::Packed<Self> {
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

impl<V> NodePacked<V> {
    #[inline]
    pub(crate) fn is_ref(self, node: node::Ref<'_, V>) -> bool {
        let ptr = match node {
            node::Ref::Node3(node) => node as *const _ as u64,
            node::Ref::Node15(node) => node as *const _ as u64,
            node::Ref::Node256(node) => node as *const _ as u64,
        };

        self.value.get() & Node::<V>::MASK_PTR == ptr
    }

    #[inline]
    pub(crate) unsafe fn into_ref_unchecked<'g>(self) -> node::Ref<'g, V> {
        #[inline]
        unsafe fn convert<'g, V, N: node::Info<V> + 'g>(ptr: u64) -> node::Ref<'g, V> {
            let node = unsafe { (ptr as *const N).as_ref() };
            validate!(node.is_some());
            N::REF(unsafe { node.unwrap_unchecked() })
        }

        let ptr = self.value.get() & Node::<V>::MASK_PTR;
        let kind = self.kind();

        if kind == node::Kind::NODE_3 {
            unsafe { convert::<_, Node3<V>>(ptr) }
        } else if kind == node::Kind::NODE_15 {
            unsafe { convert::<_, Node15<V>>(ptr) }
        } else {
            validate_eq!(kind, node::Kind::NODE_256);
            unsafe { convert::<_, Node256<V>>(ptr) }
        }
    }

    /// # SAFETY
    ///
    /// Caller must ensure there are no other references to this node.
    #[inline]
    pub(crate) unsafe fn deallocate_unchecked(self, counter: stat::Counter) {
        stat::increment(counter);

        let ptr = self.value.get() & Node::<V>::MASK_PTR;
        let kind = self.kind();

        if kind == node::Kind::NODE_3 {
            drop(Box::from_raw(ptr as *mut Node3<V>))
        } else if kind == node::Kind::NODE_15 {
            drop(Box::from_raw(ptr as *mut Node15<V>))
        } else {
            validate_eq!(kind, node::Kind::NODE_256);
            drop(Box::from_raw(ptr as *mut Node256<V>))
        }
    }
}

impl<V> Debug for NodePacked<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Node")
            .field("kind", &self.kind())
            .field("scan", &self.scan())
            .field("ptr", &(self.value.get() & Node::<V>::MASK_PTR))
            .finish()
    }
}

#[derive(ribbit::Pack)]
#[ribbit(size = 64, packed(rename = ValuePacked), eq)]
pub struct Value<V> {
    #[ribbit(size = 0)]
    _value: PhantomData<V>,

    #[ribbit(get(vis = "pub(crate)"))]
    raw: u64,
}

impl<V> Copy for Value<V> {}
impl<V> Clone for Value<V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<V> Debug for ValuePacked<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}

impl<V: value::Value> Value<V> {
    #[inline]
    pub(crate) fn from_value(value: V) -> ribbit::Packed<Self> {
        unsafe { ribbit::Packed::<Self>::new_unchecked(value.into_u64()) }
    }

    #[inline]
    pub(crate) fn from_borrow<'l>(borrow: V::Borrow<'l>) -> ribbit::Packed<Self> {
        unsafe { ribbit::Packed::<Self>::new_unchecked(V::borrow_into_u64(borrow)) }
    }
}

impl<V: value::Value> ValuePacked<V> {
    /// # SAFETY
    ///
    /// Caller must ensure there are no other references to this value.
    #[inline]
    pub(crate) unsafe fn deallocate_unchecked(self, counter: stat::Counter) {
        stat::increment(counter);
        unsafe { V::from_data(self) };
    }
}

pub(crate) struct DebugSlice<'g, V>(pub(crate) &'g [Atomic128<Edge<V>>]);

impl<V> Debug for DebugSlice<'_, V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list()
            .entries(
                self.0
                    .iter()
                    .map(|edge| edge.load_packed(Ordering::Relaxed)),
            )
            .finish()
    }
}
