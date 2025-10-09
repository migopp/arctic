use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;
use ribbit::u56;
use ribbit::u6;

use crate::byte;
use crate::node;
use crate::node::Node15;
use crate::node::Node256;
use crate::node::Node3;
use crate::stat;

#[derive(Copy, Clone, Default, Debug, ribbit::Pack)]
#[ribbit(size = 128, debug)]
pub(crate) struct Edge {
    #[ribbit(size = 64)]
    pub(crate) meta: Meta,
    #[ribbit(offset = 64)]
    pub(crate) data: u64,
}

impl Edge {
    pub(crate) const DEFAULT: ribbit::Packed<Self> = ribbit::Packed::<Self>::new(Meta::DEFAULT, 0);

    const MASK_TAG: u64 = 0b11;
    const MASK_PTR: u64 = !Self::MASK_TAG;

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

    #[inline]
    pub(crate) unsafe fn next_node_unchecked<'a>(data: u64) -> node::Ref<'a> {
        #[inline]
        unsafe fn next<'a, N: node::Info + 'a>(ptr: u64) -> node::Ref<'a> {
            let node = unsafe { (ptr as *mut N).as_ref() };
            validate!(node.is_some());
            N::REF(unsafe { node.unwrap_unchecked() })
        }

        let tag = data & Self::MASK_TAG;
        let ptr = data & Self::MASK_PTR;

        if tag == node::Kind::NODE_3 {
            unsafe { next::<Node3>(ptr) }
        } else if tag == node::Kind::NODE_15 {
            unsafe { next::<Node15>(ptr) }
        } else {
            validate_eq!(tag, node::Kind::NODE_256);
            unsafe { next::<Node256>(ptr) }
        }
    }

    /// # SAFETY
    ///
    /// Caller must ensure `edge` is a node that has no references.
    #[inline]
    pub(crate) unsafe fn deallocate_unchecked(edge: ribbit::Packed<Edge>, counter: stat::Counter) {
        let meta = edge.meta();
        let data = edge.data();

        validate!(!meta.leaf() && data != 0);

        let tag = data & Self::MASK_TAG;
        let ptr = data & Self::MASK_PTR;

        if tag == node::Kind::NODE_3 {
            drop(Box::from_raw(ptr as *mut Node3))
        } else if tag == node::Kind::NODE_15 {
            drop(Box::from_raw(ptr as *mut Node15))
        } else {
            validate_eq!(tag, node::Kind::NODE_256);
            drop(Box::from_raw(ptr as *mut Node256))
        }

        stat::increment(counter);
    }

    pub(crate) fn new_leaf(key: byte::Array, leaf: u64) -> ribbit::Packed<Self> {
        ribbit::Packed::<Self>::new(Meta::LEAF.with_key(key), leaf)
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

        let ptr = Box::leak(node) as *mut N as u64;
        let tag = N::KIND as u64;

        validate!(ptr > 0);
        validate_eq!(ptr & Self::MASK_TAG, 0);

        ribbit::Packed::<Self>::new(Meta::DEFAULT.with_key(key), ptr | tag)
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, ribbit::Pack)]
#[ribbit(size = 64, packed(rename = MetaPacked), debug, eq)]
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
