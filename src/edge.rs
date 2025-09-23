use core::sync::atomic::Ordering;

use crossbeam_epoch::Pointer as _;
use ribbit::atomic::Atomic128;

use crate::cursor;
use crate::key;
use crate::node;
use crate::node::Node15;
use crate::node::Node256;
use crate::node::Node3;
use crate::stat;

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
    pub(crate) unsafe fn retire(
        op: cursor::Op,
        guard: &crossbeam_epoch::Guard,
        edge: ribbit::Packed<Edge>,
    ) {
        let kind = edge.meta().kind();
        if kind < node::Kind::NODE_3 {
            return;
        }

        match op {
            cursor::Op::Edge(Op::Create | Op::Expand | Op::Insert | Op::Remove) => return,
            cursor::Op::Node(
                node::Op::Shrink
                | node::Op::Replace
                | node::Op::Grow
                | node::Op::Destroy
                | node::Op::Compress,
            ) => (),
        }

        let data = edge.data() as usize;
        if kind == node::Kind::NODE_3 {
            guard.defer_destroy(crossbeam_epoch::Shared::<Node3>::from_usize(data));
        } else if kind == node::Kind::NODE_15 {
            guard.defer_destroy(crossbeam_epoch::Shared::<Node15>::from_usize(data));
        } else {
            validate_eq!(kind, node::Kind::NODE_256);
            guard.defer_destroy(crossbeam_epoch::Shared::<Node256>::from_usize(data));
        }

        stat::increment(stat::Counter::Retire);
    }

    #[cold]
    pub(crate) unsafe fn deallocate(op: cursor::Op, edge: ribbit::Packed<Edge>) {
        let kind = edge.meta().kind();
        if kind < node::Kind::NODE_3 {
            return;
        }

        match op {
            cursor::Op::Node(node::Op::Destroy | node::Op::Compress)
            | cursor::Op::Edge(Op::Insert | Op::Remove) => return,

            cursor::Op::Node(node::Op::Grow | node::Op::Replace | node::Op::Shrink)
            | cursor::Op::Edge(Op::Create | Op::Expand) => (),
        }

        if kind == node::Kind::NODE_3 {
            drop(Box::from_raw(edge.data() as *mut Node3))
        } else if kind == node::Kind::NODE_15 {
            drop(Box::from_raw(edge.data() as *mut Node15))
        } else {
            validate_eq!(kind, node::Kind::NODE_256);
            drop(Box::from_raw(edge.data() as *mut Node256))
        }

        stat::increment(stat::Counter::Deallocate);
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
