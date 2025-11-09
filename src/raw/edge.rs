mod be;

pub(crate) use be::Be;

use core::fmt::Debug;
use core::marker::PhantomData;
use core::num::NonZeroU32;
use core::num::NonZeroU64;
use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;
use ribbit::OptionExt as _;

use crate::raw::node;
use crate::raw::node::Node15;
use crate::raw::node::Node256;
use crate::raw::node::Node3;
use crate::stat;

#[derive(Copy, Clone, Default, ribbit::Pack)]
#[ribbit(size = 128, packed(rename = EdgePacked))]
pub struct Edge<M> {
    #[ribbit(size = 64)]
    pub(crate) meta: M,

    data: u64,
}

impl<M: Meta> Edge<M> {
    pub(crate) const DEFAULT: ribbit::Packed<Self> = ribbit::Packed::<Self>::new(M::DEFAULT, 0);

    #[inline]
    pub(crate) fn freeze(edge: &Atomic128<Self>) {
        let mut old = edge.load_packed(Ordering::Relaxed);

        while !M::is_frozen(old.meta()) {
            match edge.compare_exchange_packed(
                old,
                old.with_meta(M::with_frozen(old.meta(), true)),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(conflict) => old = conflict,
            }
        }
    }

    #[cold]
    pub(crate) fn new_node<N, I>(meta: ribbit::Packed<M>, edges: I) -> ribbit::Packed<Self>
    where
        N: node::Node<M>,
        I: IntoIterator<Item = (u8, ribbit::Packed<Edge<M>>)>,
    {
        validate!(!M::is_frozen(meta));

        let mut node = Box::new(N::default());

        for (key, edge) in edges {
            node.reserve(key)
                .expect("Node can fit all edges")
                .set_packed(edge);
        }

        ribbit::Packed::<Self>::new(M::with_value(meta, false), Node::new(node).value.get())
    }

    pub(crate) fn new_value(meta: ribbit::Packed<M>, value: u64) -> ribbit::Packed<Self> {
        validate!(!M::is_frozen(meta));

        ribbit::Packed::<Self>::new(M::with_value(meta, true), value)
    }
}

impl<M: Meta> EdgePacked<M> {
    #[inline]
    pub(crate) fn is_null(self) -> bool {
        !M::is_value(self.meta()) && self.data() == 0
    }

    #[inline]
    pub(crate) fn as_node(self) -> Option<ribbit::Packed<Node<M>>> {
        if M::is_value(self.meta()) {
            return None;
        }

        unsafe { ribbit::Packed::<Option<Node<M>>>::new_unchecked(self.data()) }
    }

    #[inline]
    pub(crate) fn as_value(self) -> Option<u64> {
        M::is_value(self.meta()).then(|| self.data())
    }

    #[inline]
    pub(crate) fn into_raw(self) -> u64 {
        self.data()
    }

    #[inline]
    pub(crate) fn child(self) -> Option<Child<M>> {
        let data = self.data();
        if M::is_value(self.meta()) {
            Some(Child::Value(data))
        } else {
            unsafe { ribbit::Packed::<Option<Node<M>>>::new_unchecked(data) }.map(Child::Node)
        }
    }

    #[inline]
    pub(crate) fn with_node(self, node: ribbit::Packed<Node<M>>) -> Self {
        validate!(!M::is_value(self.meta()));
        self.with_data(node.value.get())
    }

    #[inline]
    pub(crate) fn with_value(self, value: u64) -> Self {
        validate!(M::is_value(self.meta()));
        self.with_data(value)
    }

    #[inline]
    pub(crate) fn unfreeze(self) -> Self {
        self.with_meta(M::with_frozen(self.meta(), false))
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

impl<M: Meta> Debug for EdgePacked<M>
where
    M::Packed: core::fmt::Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut debug = f.debug_struct("Edge");

        debug.field("meta", &self.meta());
        debug.field("data", &self.child());

        debug.finish()
    }
}

pub(crate) trait Meta: ribbit::Pack {
    const DEFAULT: ribbit::Packed<Self>;

    const MAX_LEN: Self::Len;

    type Len: Copy + Eq;

    fn len(meta: ribbit::Packed<Self>) -> Self::Len;
    fn len_to_bits(len: Self::Len) -> usize;

    fn equal(left: ribbit::Packed<Self>, right: ribbit::Packed<Self>) -> bool;
    fn cmp(left: ribbit::Packed<Self>, right: ribbit::Packed<Self>) -> core::cmp::Ordering;

    fn is_value(meta: ribbit::Packed<Self>) -> bool;
    fn is_frozen(meta: ribbit::Packed<Self>) -> bool;

    fn with_frozen(meta: ribbit::Packed<Self>, frozen: bool) -> ribbit::Packed<Self>;
    fn with_value(meta: ribbit::Packed<Self>, value: bool) -> ribbit::Packed<Self>;

    fn expand(
        old: ribbit::Packed<Self>,
        new: ribbit::Packed<Self>,
    ) -> Result<(ribbit::Packed<Self>, u8, ribbit::Packed<Self>), ()>;

    fn compress(
        parent: ribbit::Packed<Self>,
        byte: u8,
        child: ribbit::Packed<Self>,
    ) -> Option<ribbit::Packed<Self>>;
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

impl<M> Node<M> {
    const MASK_TAG: u64 = 0b111;
    const MASK_PTR: u64 = !Self::MASK_TAG;
}

impl<M> Node<M>
where
    M: Meta,
{
    #[inline]
    fn new<N: node::Node<M>>(node: Box<N>) -> ribbit::Packed<Self> {
        let ptr = NonNull::from(Box::leak(node));
        let kind = N::KIND as u64;

        validate_eq!(ptr.addr().get() as u64 & Self::MASK_TAG, 0);

        unsafe {
            ribbit::Packed::<Self>::new_unchecked(NonZeroU64::new_unchecked(
                kind | ptr.addr().get() as u64,
            ))
        }
    }

    pub(crate) fn new_unchecked(raw: u64) -> ribbit::Packed<Self> {
        let node = unsafe { ribbit::Packed::<Option<Node<M>>>::new_unchecked(raw) };
        if cfg!(feature = "validate") {
            node.unwrap()
        } else {
            unsafe { node.unwrap_unchecked() }
        }
    }
}

impl<M> NodePacked<M>
where
    M: Meta,
{
    #[inline]
    pub(crate) fn is_ref(self, node: node::Ref<'_, M>) -> bool {
        let ptr = match node {
            node::Ref::Node3(node) => node as *const _ as u64,
            node::Ref::Node15(node) => node as *const _ as u64,
            node::Ref::Node256(node) => node as *const _ as u64,
        };

        self.value.get() & Node::<M>::MASK_PTR == ptr
    }

    #[inline]
    pub(crate) unsafe fn into_ref_unchecked<'g>(self) -> node::Ref<'g, M> {
        #[inline]
        unsafe fn as_ref<'g, M, N>(ptr: u64) -> &'g N
        where
            M: Meta,
            N: node::Node<M> + 'g,
        {
            let node = unsafe { (ptr as *const N).as_ref() };
            validate!(node.is_some());
            unsafe { node.unwrap_unchecked() }
        }

        let ptr = self.value.get() & Node::<M>::MASK_PTR;
        let kind = self.kind();

        if kind == node::Kind::NODE_3 {
            node::Ref::Node3(unsafe { as_ref::<_, Node3<M>>(ptr) })
        } else if kind == node::Kind::NODE_15 {
            node::Ref::Node15(unsafe { as_ref::<_, Node15<M>>(ptr) })
        } else {
            validate_eq!(kind, node::Kind::NODE_256);
            node::Ref::Node256(unsafe { as_ref::<_, Node256<M>>(ptr) })
        }
    }

    /// # SAFETY
    ///
    /// Caller must ensure there are no other references to this node.
    #[inline]
    pub(crate) unsafe fn deallocate_unchecked(self, counter: stat::Counter) {
        stat::increment(counter);

        let ptr = self.value.get() & Node::<M>::MASK_PTR;
        let kind = self.kind();

        if kind == node::Kind::NODE_3 {
            drop(Box::from_raw(ptr as *mut Node3<M>))
        } else if kind == node::Kind::NODE_15 {
            drop(Box::from_raw(ptr as *mut Node15<M>))
        } else {
            validate_eq!(kind, node::Kind::NODE_256);
            drop(Box::from_raw(ptr as *mut Node256<M>))
        }
    }
}

impl<M> Debug for NodePacked<M> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Node")
            .field("kind", &self.kind())
            .field("scan", &self.scan())
            .field("ptr", &(self.value.get() & Node::<M>::MASK_PTR))
            .finish()
    }
}
