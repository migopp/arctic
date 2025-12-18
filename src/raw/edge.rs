mod be;
mod le;

pub(crate) use be::Be;
pub(crate) use le::Le;
use ribbit::u6;

use core::fmt::Debug;
use core::marker::PhantomData;
use core::num::NonZeroU32;
use core::num::NonZeroU64;
use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use ribbit::Atomic;
use ribbit::OptionExt as _;

use crate::raw::key;
use crate::raw::node;
use crate::raw::node::Node as _;
use crate::raw::node::Node15;
use crate::raw::node::Node256;
use crate::raw::node::Node3;
use crate::raw::node::Node47;
use crate::stat;

#[derive(Copy, Clone, Default, ribbit::Pack)]
#[ribbit(size = 128, packed(rename = EdgePacked))]
pub(crate) struct Edge<M> {
    #[ribbit(size = 64)]
    pub(crate) meta: M,

    data: u64,
}

impl<M: ribbit::Pack<Packed: Meta>> Edge<M> {
    pub(crate) const DEFAULT: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(<M::Packed as Meta>::DEFAULT, 0);

    #[inline]
    pub(crate) fn freeze(edge: &Atomic<Self>) {
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

    pub(crate) fn new_path<R>(mut reader: R, value: u64) -> ribbit::Packed<Self>
    where
        R: key::Read<Edge = M>,
    {
        let key = reader.read(<<R::Edge as ribbit::Pack>::Packed as Meta>::MAX_LEN);
        let Some(mut byte) = reader.next() else {
            return Self::new_value(key, value);
        };

        let mut tail = NonNull::from(Box::leak(Box::new(Node3::default())));
        let head =
            ribbit::Packed::<Self>::new(key.with_value(false), Ptr::new_ptr(tail).value.get());

        loop {
            let edge = unsafe { tail.as_mut().insert(byte) };
            let edge = if cfg!(feature = "validate") {
                edge.expect("Node3 fits one edge")
            } else {
                unsafe { edge.unwrap_unchecked() }
            };

            let key = reader.read(<<R::Edge as ribbit::Pack>::Packed as Meta>::MAX_LEN);

            let Some(next_byte) = reader.next() else {
                edge.set_packed(Self::new_value(key, value));
                return head;
            };
            byte = next_byte;

            let next_node = NonNull::from(Box::leak(Box::new(Node3::default())));
            edge.set_packed(ribbit::Packed::<Self>::new(
                key.with_value(false),
                Ptr::new_ptr(next_node).value.get(),
            ));
            tail = next_node;
        }
    }

    #[cold]
    pub(crate) fn new_node<N, K, E>(
        key: <<M as ribbit::Pack>::Packed as Meta>::Key,
        keys: K,
        edges: E,
    ) -> ribbit::Packed<Self>
    where
        N: node::Node<M>,
        K: IntoIterator<Item = u8>,
        E: IntoIterator<Item = ribbit::Packed<Edge<M>>>,
    {
        unsafe { Self::new_node_unchecked::<N, K, E>(key.with_value(false), keys, edges) }
    }

    #[cold]
    pub(crate) unsafe fn new_node_unchecked<N, K, E>(
        meta: ribbit::Packed<M>,
        keys: K,
        edges: E,
    ) -> ribbit::Packed<Self>
    where
        N: node::Node<M>,
        K: IntoIterator<Item = u8>,
        E: IntoIterator<Item = ribbit::Packed<Edge<M>>>,
    {
        validate!(!meta.is_frozen());
        validate!(!meta.is_value());

        let mut node = Box::new(N::default());

        for (key, edge) in keys.into_iter().zip(edges) {
            node.insert(key)
                .expect("Node can fit all edges")
                .set_packed(edge);
        }

        ribbit::Packed::<Self>::new(meta, Ptr::new(node).value.get())
    }

    pub(crate) fn new_value(
        meta: <<M as ribbit::Pack>::Packed as Meta>::Key,
        value: u64,
    ) -> ribbit::Packed<Self> {
        ribbit::Packed::<Self>::new(meta.with_value(true), value)
    }
}

impl<M: ribbit::Pack<Packed: Meta>> EdgePacked<M> {
    #[inline]
    pub(crate) fn is_null(self) -> bool {
        !self.meta().is_value() && self.data() == 0
    }

    #[inline]
    pub(crate) fn as_node(self) -> Option<ribbit::Packed<Ptr<M>>> {
        if self.meta().is_value() {
            return None;
        }

        unsafe { ribbit::Packed::<Option<Ptr<M>>>::new_unchecked(self.data()) }
    }

    #[inline]
    pub(crate) fn as_value(self) -> Option<u64> {
        self.meta().is_value().then(|| self.data())
    }

    #[inline]
    pub(crate) fn into_raw(self) -> u64 {
        self.data()
    }

    #[inline]
    pub(crate) fn child(self) -> Option<Child<M>> {
        let data = self.data();
        if self.meta().is_value() {
            Some(Child::Value(data))
        } else {
            unsafe { ribbit::Packed::<Option<Ptr<M>>>::new_unchecked(data) }.map(Child::Node)
        }
    }

    #[inline]
    pub(crate) fn with_node(self, node: ribbit::Packed<Ptr<M>>) -> Self {
        validate!(!self.meta().is_value());
        self.with_data(node.value.get())
    }

    #[inline]
    pub(crate) fn with_value(self, value: u64) -> Self {
        validate!(self.meta().is_value());
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

impl<M: ribbit::Pack> Debug for EdgePacked<M>
where
    M::Packed: Meta + core::fmt::Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut debug = f.debug_struct("Edge");

        debug.field("meta", &self.meta());
        debug.field("data", &self.child());

        debug.finish()
    }
}

pub(crate) trait Meta: ribbit::Unpack + core::fmt::Debug {
    const DEFAULT: Self;

    const MAX_LEN: Self::Len;

    type Len: Len;
    type Key: Key<Meta = Self, Len = Self::Len>;

    fn key(self) -> Self::Key;

    fn is_value(self) -> bool;
    fn is_frozen(self) -> bool;

    fn with_frozen(self, frozen: bool) -> Self;

    fn expand(self, new: Self::Key) -> Result<(Self::Key, u8, Self), ()>;

    fn compress(self, byte: u8, child: Self) -> Option<Self>;
}

pub(crate) trait Key: Copy + Eq + Ord + core::fmt::Debug {
    type Meta;
    type Len: Len;

    fn len(self) -> Self::Len;
    fn with_value(self, value: bool) -> Self::Meta;
    fn prefix(self, len: Self::Len) -> Self;

    #[cfg_attr(not(test), expect(unused))]
    fn with_bytes<F: FnOnce(&[u8]) -> T, T>(self, apply: F) -> T;
}

pub(crate) trait Len: Copy + Eq {
    #[cfg_attr(not(test), expect(unused))]
    fn new(bits: usize) -> Self;
    fn bits(self) -> usize;
}

impl Len for u6 {
    #[inline]
    fn new(bits: usize) -> Self {
        validate_eq!(bits & 0b111, 0);
        validate!(bits <= u8::MAX as usize);
        u6::new(bits as u8)
    }

    #[inline]
    fn bits(self) -> usize {
        self.value() as usize
    }
}

/// Edge-related structural modification operation. Does not require freezing.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Smo {
    /// Node creation
    Create,

    /// Path expansion
    Expand,
}

impl Smo {
    /// Whether this operation allocates a new node.
    #[inline]
    pub(crate) fn is_allocate(self) -> bool {
        true
    }
}

pub(crate) enum Child<C> {
    Node(ribbit::Packed<Ptr<C>>),
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
#[ribbit(size = 64, packed(rename = PtrPacked), eq, nonzero)]
pub(crate) struct Ptr<C> {
    #[ribbit(size = 2, get(vis = "pub(crate)"))]
    kind: node::Kind,

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
    M: ribbit::Pack<Packed: Meta>,
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

    fn new_ptr(ptr: NonNull<Node3<M>>) -> ribbit::Packed<Self> {
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
    M: ribbit::Pack<Packed: Meta>,
{
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
            validate_eq!(kind, node::Kind::NODE_256);
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
            validate_eq!(kind, node::Kind::NODE_256);
            unsafe { as_ref::<_, Node256<M>>(ptr) }.get_or_insert(key)
        }
    }

    #[inline]
    pub(crate) unsafe fn replace_unchecked(
        self,
        parent: ribbit::Packed<M>,
    ) -> (node::Smo, ribbit::Packed<Edge<M>>) {
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
            validate_eq!(kind, node::Kind::NODE_256);
            unsafe { as_ref::<_, Node256<M>>(ptr) }.replace::<256>(parent)
        }
    }

    #[inline]
    pub(crate) unsafe fn entries_unchecked<'g, L: node::Lower, U: node::Upper>(
        self,
        lower: L,
        upper: U,
    ) -> node::NodeIter<'g, L, U, M> {
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
            validate_eq!(kind, node::Kind::NODE_256);
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
            validate_eq!(kind, node::Kind::NODE_256);
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
                validate_eq!(kind, node::Kind::NODE_256);
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
    M: ribbit::Pack<Packed: Meta>,
    N: node::Node<M> + 'g,
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
