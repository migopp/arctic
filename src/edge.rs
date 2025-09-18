use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;
use ribbit::Unpack as _;

use crate::key;
use crate::node;
use crate::node::Node15;
use crate::node::Node256;
use crate::node::Node3;
use crate::Or;

#[ribbit::pack(size = 128)]
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

    #[inline]
    pub(crate) unsafe fn next<'a>(edge: ribbit::Packed<Edge>) -> Option<Or<u64, node::Ref<'a>>> {
        let node = match edge.meta().kind().unpack() {
            node::Kind::None => return None,
            node::Kind::Leaf => return Some(Or::L(edge.data())),
            node::Kind::Node3 => unsafe { (edge.data() as *mut Node3).as_ref() }
                .map(node::Ref::Node3)
                .map(Or::R),
            node::Kind::Node15 => unsafe { (edge.data() as *mut Node15).as_ref() }
                .map(node::Ref::Node15)
                .map(Or::R),
            node::Kind::Node256 => unsafe { (edge.data() as *mut Node256).as_ref() }
                .map(node::Ref::Node256)
                .map(Or::R),
        };

        Some(match cfg!(feature = "validate") {
            true => node.unwrap(),
            false => unsafe { node.unwrap_unchecked() },
        })
    }

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

#[derive(Copy, Clone, Debug)]
pub(crate) enum Op {
    /// Node creation
    Create,

    /// Path expansion
    Expand,

    /// Leaf insertion
    Insert,

    /// Leaf removal
    Remove,
}
