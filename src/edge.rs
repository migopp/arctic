use core::fmt::Debug;
use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;
use ribbit::u56;
use ribbit::u6;
use ribbit::u61;
use ribbit::Unpack as _;

use crate::byte;
use crate::node;
use crate::node::Node15;
use crate::node::Node256;
use crate::node::Node3;
use crate::stat;
use crate::Value;

#[derive(ribbit::Pack)]
#[ribbit(size = 128, packed(rename = EdgePacked))]
pub struct Edge<V> {
    #[ribbit(size = 64)]
    pub(crate) meta: Meta,
    #[ribbit(size = 64)]
    pub(crate) data: Data<V>,
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
            meta: Meta::default(),
            data: Data::default(),
        }
    }
}

impl<V> Edge<V> {
    pub(crate) const DEFAULT: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(Meta::DEFAULT, Data::DEFAULT);

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

        ribbit::Packed::<Self>::new(Meta::DEFAULT.with_key(key), Data::from_node(node))
    }
}

impl<V> EdgePacked<V> {
    #[inline]
    pub(crate) fn is_node(self) -> bool {
        !self.meta().is_value() && !self.data().is_null()
    }

    #[inline]
    pub(crate) fn is_null(self) -> bool {
        !self.meta().is_value() && self.data().is_null()
    }

    #[inline]
    pub(crate) fn is_scan(self) -> bool {
        !self.meta().is_value() && self.data().scan()
    }

    #[inline]
    pub(crate) fn unfreeze(self) -> Self {
        self.with_meta(self.meta().with_frozen(false))
    }
}

impl<V: Value> EdgePacked<V> {
    /// # SAFETY
    ///
    /// Caller must ensure there are no references to the child of this edge,
    /// and that the child is non-null.
    #[inline]
    pub(crate) unsafe fn deallocate_unchecked(self, counter: stat::Counter) {
        validate!(!self.is_null());
        let data = self.data();
        if self.meta().is_value() {
            data.deallocate_value_unchecked(counter);
        } else {
            data.deallocate_node_unchecked(counter);
        }
    }
}

impl<V> Debug for EdgePacked<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut debug = f.debug_struct("Edge");

        debug.field("meta", &self.meta());

        if self.meta().is_value() {
            debug.field("value", &self.data().value);
        } else {
            debug.field("node", &self.data());
        }

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

#[derive(ribbit::Pack)]
#[ribbit(size = 64, packed(rename = DataPacked), eq)]
pub struct Data<V> {
    #[ribbit(size = 0)]
    _value: PhantomData<V>,

    #[ribbit(size = 2)]
    kind: node::Kind,

    pub(crate) scan: bool,

    #[ribbit(with(skip))]
    _placeholder_data: u61,
}

impl<V> Copy for Data<V> {}
impl<V> Clone for Data<V> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<V> Default for Data<V> {
    fn default() -> Self {
        Self::DEFAULT.unpack()
    }
}

impl<V> Data<V> {
    const DEFAULT: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(node::Kind::NODE_3, false, u61::new(0));

    const MASK_TAG: u64 = 0b111;
    const MASK_PTR: u64 = !Self::MASK_TAG;

    #[inline]
    fn from_node<N: node::Info<V>>(node: Box<N>) -> ribbit::Packed<Self> {
        let ptr = Box::leak(node) as *mut N as u64;
        let kind = N::KIND as u64;

        validate!(ptr > 0);
        validate_eq!(ptr & Self::MASK_TAG, 0);

        unsafe { ribbit::Packed::<Self>::new_unchecked(kind | ptr) }
    }
}

impl<V: Value> Data<V> {
    #[inline]
    pub(crate) fn from_value(value: V) -> ribbit::Packed<Self> {
        unsafe { ribbit::Packed::<Self>::new_unchecked(value.into_u64()) }
    }

    #[inline]
    pub(crate) fn from_borrow<'l>(borrow: V::Borrow<'l>) -> ribbit::Packed<Self> {
        unsafe { ribbit::Packed::<Self>::new_unchecked(V::borrow_into_u64(borrow)) }
    }
}

impl<V: Value> DataPacked<V> {
    /// # SAFETY
    ///
    /// Caller must ensure this is a value, and that there are no other references to it.
    #[inline]
    pub(crate) unsafe fn deallocate_value_unchecked(self, counter: stat::Counter) {
        stat::increment(counter);
        unsafe { V::from_data(self) };
    }
}

impl<V> DataPacked<V> {
    pub(crate) fn value(self) -> u64 {
        self.value
    }

    #[inline]
    pub(crate) fn is_null(self) -> bool {
        self.value == 0
    }

    #[inline]
    pub(crate) fn is_ref(self, node: node::Ref<'_, V>) -> bool {
        if self.is_null() {
            return false;
        }

        let ptr = match node {
            node::Ref::Node3(node) => node as *const _ as u64,
            node::Ref::Node15(node) => node as *const _ as u64,
            node::Ref::Node256(node) => node as *const _ as u64,
        };

        self.value & Data::<V>::MASK_PTR == ptr
    }

    #[inline]
    pub(crate) unsafe fn into_node_unchecked<'g>(self) -> node::Ref<'g, V> {
        #[inline]
        unsafe fn convert<'g, V, N: node::Info<V> + 'g>(ptr: u64) -> node::Ref<'g, V> {
            let node = unsafe { (ptr as *const N).as_ref() };
            validate!(node.is_some());
            N::REF(unsafe { node.unwrap_unchecked() })
        }

        validate!(!self.is_null());

        let ptr = self.value & Data::<V>::MASK_PTR;
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
    /// Caller must ensure this is a non-null node, and that there
    /// are no other references to it.
    #[inline]
    pub(crate) unsafe fn deallocate_node_unchecked(self, counter: stat::Counter) {
        validate!(!self.is_null());
        stat::increment(counter);

        let ptr = self.value & Data::<V>::MASK_PTR;
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

impl<V> Debug for DataPacked<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Data")
            .field("kind", &self.kind())
            .field("scan", &self.scan())
            .field("ptr", &(self.value & Data::<V>::MASK_PTR))
            .finish()
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
