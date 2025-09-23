use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;
use ribbit::Unpack as _;

use crate::key;
use crate::node;
use crate::node::Node15;
use crate::node::Node256;
use crate::node::Node3;

#[ribbit::pack(size = 128, debug)]
#[derive(Copy, Clone, Default, Debug)]
pub(crate) struct Edge {
    #[ribbit(size = 63)]
    pub(crate) meta: Meta,
    #[ribbit(offset = 64)]
    pub(crate) data: u64,
}

impl Edge {
    pub(crate) const DEFAULT: ribbit::Packed<Self> = ribbit::Packed::<Self>::new(Meta::DEFAULT, 0);

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

    /// # SAFETY
    /// Caller must ensure that:
    /// - `data` and `kind` were loaded atomically from the same edge
    /// - `kind >= node::Kind::NODE_3`
    #[inline]
    pub(crate) unsafe fn next_node_unchecked<'a>(
        data: u64,
        kind: ribbit::Packed<node::Kind>,
    ) -> node::Ref<'a> {
        #[inline]
        unsafe fn next<'a, N: node::Info + 'a>(data: u64) -> node::Ref<'a> {
            let node = unsafe { (data as *mut N).as_ref() };
            validate!(node.is_some());
            N::REF(unsafe { node.unwrap_unchecked() })
        }

        if kind == node::Kind::NODE_3 {
            unsafe { next::<Node3>(data) }
        } else if kind == node::Kind::NODE_15 {
            unsafe { next::<Node15>(data) }
        } else {
            validate_eq!(kind, node::Kind::NODE_256);
            unsafe { next::<Node256>(data) }
        }
    }

    #[cold]
    pub(crate) unsafe fn deallocate(edge: ribbit::Packed<Edge>) {
        match edge.meta().kind().unpack() {
            node::Kind::None | node::Kind::Leaf => {
                unreachable!()
            }
            node::Kind::Node3 => drop(Box::from_raw(edge.data() as *mut Node3)),
            node::Kind::Node15 => drop(Box::from_raw(edge.data() as *mut Node15)),
            node::Kind::Node256 => drop(Box::from_raw(edge.data() as *mut Node256)),
        }
    }

    pub(crate) fn new_leaf(key: ribbit::Packed<key::Array>, leaf: u64) -> ribbit::Packed<Self> {
        ribbit::Packed::<Self>::new(Meta::LEAF.with_key(key), leaf)
    }

    #[cold]
    pub(crate) fn new_node<N, I>(key: ribbit::Packed<key::Array>, edges: I) -> ribbit::Packed<Self>
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

        let node = Box::leak(node) as *mut N;
        ribbit::Packed::<Self>::new(N::META.with_key(key), node as u64)
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[ribbit::pack(size = 63, debug, eq)]
pub(crate) struct Meta {
    #[ribbit(size = 59)]
    pub(crate) key: key::Array,
    pub(crate) frozen: bool,
    #[ribbit(size = 3)]
    pub(crate) kind: node::Kind,
}

impl Meta {
    pub(crate) const DEFAULT: ribbit::Packed<Self> = ribbit::Packed::<Self>::new(
        key::Array::EMPTY,
        false,
        ribbit::Packed::<node::Kind>::new_none(),
    );

    const LEAF: ribbit::Packed<Self> =
        Self::DEFAULT.with_kind(ribbit::Packed::<node::Kind>::new_leaf());
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
