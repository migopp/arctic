mod be;
mod le;

pub(crate) use be::Be;
pub(crate) use le::Le;
use ribbit::u6;

use core::fmt::Debug;
use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use ribbit::Atomic;
use ribbit::OptionExt as _;

use crate::raw::key;
use crate::raw::node;
use crate::raw::node::Node as _;
use crate::raw::node::Node3;
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

    #[inline]
    pub(crate) fn new_path<R>(mut reader: R, value: u64) -> ribbit::Packed<Self>
    where
        R: key::Read<Edge = M>,
    {
        let key = reader.read(<<R::Edge as ribbit::Pack>::Packed as Meta>::MAX_LEN);
        let Some(byte) = reader.next() else {
            return Self::new_value(key, value);
        };
        Self::new_path_cold(reader, key, byte, value)
    }

    #[cold]
    fn new_path_cold<R>(
        mut reader: R,
        key: <<R::Edge as ribbit::Pack>::Packed as Meta>::Key,
        mut byte: u8,
        value: u64,
    ) -> ribbit::Packed<Self>
    where
        R: key::Read<Edge = M>,
    {
        let mut tail = NonNull::from(Box::leak(Box::new(Node3::default())));
        let head = ribbit::Packed::<Self>::new(
            key.with_value(false),
            node::Ptr::new_ptr(tail).raw().get(),
        );

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
                node::Ptr::new_ptr(next_node).raw().get(),
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

        ribbit::Packed::<Self>::new(meta, node::Ptr::new(node).raw().get())
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
    pub(crate) fn as_node(self) -> Option<ribbit::Packed<node::Ptr<M>>> {
        if self.meta().is_value() {
            return None;
        }

        unsafe { ribbit::Packed::<Option<node::Ptr<M>>>::new_unchecked(self.data()) }
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
            unsafe { ribbit::Packed::<Option<node::Ptr<M>>>::new_unchecked(data) }.map(Child::Node)
        }
    }

    #[inline]
    pub(crate) fn with_node(self, node: ribbit::Packed<node::Ptr<M>>) -> Self {
        validate!(!self.meta().is_value());
        self.with_data(node.raw().get())
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
            Some(Child::Node(node)) => unsafe { node.deallocate(counter) },
            Some(Child::Value(value)) => deallocate_value(value),
        }
    }

    #[inline]
    pub(crate) unsafe fn deallocate_recursive_unchecked(self, counter: stat::Counter) {
        match self.child() {
            None if cfg!(feature = "validate") => unreachable!(),
            None => unsafe { core::hint::unreachable_unchecked() },
            Some(Child::Node(node)) => unsafe { node.deallocate_recursive(counter) },
            Some(Child::Value(_)) => (),
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

pub(crate) trait Key: Copy + Eq + Ord + core::fmt::Debug + IntoIterator<Item = u8> {
    type Meta;
    type Len: Len;

    fn len(self) -> Self::Len;
    fn with_value(self, value: bool) -> Self::Meta;
    fn prefix(self, len: Self::Len) -> Self;
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

pub(crate) enum Child<M> {
    Node(ribbit::Packed<node::Ptr<M>>),
    Value(u64),
}

impl<M> Debug for Child<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Node(node) => f.debug_tuple("Node").field(node).finish(),
            Self::Value(value) => f.debug_tuple("Value").field(value).finish(),
        }
    }
}
