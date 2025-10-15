use core::fmt::Debug;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;
use ribbit::u56;
use ribbit::u6;
use ribbit::u61;

use crate::byte;
use crate::node;
use crate::node::Node15;
use crate::node::Node256;
use crate::node::Node3;
use crate::stat;

#[derive(Copy, Clone, Default, ribbit::Pack)]
#[ribbit(size = 128, packed(rename = EdgePacked))]
pub(crate) struct Edge {
    #[ribbit(size = 64)]
    pub(crate) meta: Meta,
    #[ribbit(size = 64)]
    pub(crate) data: Data,
}

impl Edge {
    pub(crate) const DEFAULT: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(Meta::DEFAULT, Data::DEFAULT);

    #[inline]
    pub(crate) fn new_leaf(key: byte::Array, leaf: u64) -> ribbit::Packed<Self> {
        ribbit::Packed::<Self>::new(Meta::LEAF.with_key(key), Data::from_leaf(leaf))
    }

    #[cold]
    pub(crate) fn new_node<N, I>(key: byte::Array, edges: I) -> ribbit::Packed<Self>
    where
        N: node::Info,
        I: IntoIterator<Item = (u8, ribbit::Packed<Edge>)>,
    {
        let mut node = Box::new(N::default());

        for (key, edge) in edges {
            node.reserve(key)
                .expect("Node can fit all edges")
                .set_packed(edge);
        }

        ribbit::Packed::<Self>::new(Meta::DEFAULT.with_key(key), Data::from_node(node))
    }

    #[inline]
    pub(crate) fn freeze(edge: &Atomic128<Self>) {
        let mut old = edge.load_packed(Ordering::Relaxed);

        while !old.meta().frozen() {
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
}

impl EdgePacked {
    #[inline]
    pub(crate) fn is_node(self) -> bool {
        !self.meta().leaf() && !self.data().is_null()
    }

    #[inline]
    pub(crate) fn is_null(self) -> bool {
        !self.meta().leaf() && self.data().is_null()
    }

    #[inline]
    pub(crate) fn is_scan(self) -> bool {
        !self.meta().leaf() && self.data().scan()
    }
}

impl Debug for EdgePacked {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut debug = f.debug_struct("Edge");

        debug.field("meta", &self.meta());

        if self.meta().leaf() {
            debug.field("leaf", &self.data().value);
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
    pub(crate) leaf: bool,
    pub(crate) frozen: bool,
    #[ribbit(with(skip))]
    _placeholder_array: u56,
}

impl Meta {
    pub(crate) const DEFAULT: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(u6::new(0), false, false, u56::new(0));

    const LEAF: ribbit::Packed<Self> = Self::DEFAULT.with_leaf(true);
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
}

impl Debug for MetaPacked {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Meta")
            .field("leaf", &self.leaf())
            .field("frozen", &self.frozen())
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

    /// Leaf insertion
    Insert,

    /// Leaf removal
    #[expect(dead_code)]
    Remove,
}

#[derive(Copy, Clone, Default, ribbit::Pack)]
#[ribbit(size = 64, packed(rename = DataPacked))]
pub(crate) struct Data {
    #[ribbit(size = 2)]
    kind: node::Kind,

    #[ribbit(get(vis = "pub(crate)"))]
    scan: bool,

    #[ribbit(with(skip))]
    _placeholder_data: u61,
}

impl Data {
    const DEFAULT: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(node::Kind::NODE_3, false, u61::new(0));

    #[inline]
    fn from_leaf(value: u64) -> ribbit::Packed<Self> {
        unsafe { ribbit::Packed::<Self>::new_unchecked(value) }
    }

    #[inline]
    fn from_node<N: node::Info>(node: Box<N>) -> ribbit::Packed<Self> {
        let ptr = Box::leak(node) as *mut N as u64;
        let kind = N::KIND as u64;

        validate!(ptr > 0);
        validate_eq!(ptr & ribbit::Packed::<Self>::MASK_TAG, 0);

        unsafe { ribbit::Packed::<Self>::new_unchecked(ptr | kind) }
    }
}

impl DataPacked {
    const MASK_TAG: u64 = 0b111;
    const MASK_PTR: u64 = !Self::MASK_TAG;

    #[inline]
    pub(crate) fn is_null(self) -> bool {
        self.value == 0
    }

    #[inline]
    pub(crate) fn into_leaf(self) -> u64 {
        self.value
    }

    #[inline]
    pub(crate) unsafe fn into_node_unchecked<'a>(self) -> node::Ref<'a> {
        #[inline]
        unsafe fn convert<'a, N: node::Info>(ptr: u64) -> node::Ref<'a> {
            let node = unsafe { (ptr as *const N).as_ref() };
            validate!(node.is_some());
            N::REF(unsafe { node.unwrap_unchecked() })
        }

        validate!(!self.is_null());

        let ptr = self.value & Self::MASK_PTR;
        let kind = self.kind();

        if kind == node::Kind::NODE_3 {
            unsafe { convert::<Node3>(ptr) }
        } else if kind == node::Kind::NODE_15 {
            unsafe { convert::<Node15>(ptr) }
        } else {
            validate_eq!(kind, node::Kind::NODE_256);
            unsafe { convert::<Node256>(ptr) }
        }
    }

    /// # SAFETY
    ///
    /// Caller must ensure there are no references to this node.
    #[inline]
    pub(crate) unsafe fn deallocate_unchecked(self, counter: stat::Counter) {
        validate!(!self.is_null());

        let ptr = self.value & Self::MASK_PTR;
        let kind = self.kind();

        if kind == node::Kind::NODE_3 {
            drop(Box::from_raw(ptr as *mut Node3))
        } else if kind == node::Kind::NODE_15 {
            drop(Box::from_raw(ptr as *mut Node15))
        } else {
            validate_eq!(kind, node::Kind::NODE_256);
            drop(Box::from_raw(ptr as *mut Node256))
        }

        stat::increment(counter);
    }
}

impl Debug for DataPacked {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Data")
            .field("kind", &self.kind())
            .field("scan", &self.scan())
            .field("ptr", &(self.value & Self::MASK_PTR))
            .finish()
    }
}

pub(crate) struct DebugSlice<'a>(pub(crate) &'a [Atomic128<Edge>]);

impl Debug for DebugSlice<'_> {
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
