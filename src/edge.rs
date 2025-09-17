use core::ops::Deref;
use core::ops::DerefMut;
use core::sync::atomic::Ordering;

use crate::key;
use crate::node;
use crate::node::Node15;
use crate::node::Node256;
use crate::node::Node3;
use crate::Or;
use crate::Split;

#[repr(transparent)]
#[derive(Default, Debug)]
pub(crate) struct Edge(Split<Meta, Data>);

impl Edge {
    pub(crate) fn freeze(&self) {
        let mut old_meta = self.load_low_packed(Ordering::Relaxed);

        if old_meta.frozen() {
            return;
        }

        let mut old_data = self.load_high_packed(Ordering::Relaxed);

        loop {
            match self.compare_exchange_packed(
                (old_meta, old_data),
                (old_meta.with_frozen(true), old_data),
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err((meta, _)) if meta.frozen() => break,
                Err((meta, data)) => {
                    old_meta = meta;
                    old_data = data;
                }
            }
        }
    }
}

impl Deref for Edge {
    type Target = Split<Meta, Data>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Edge {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[ribbit::pack(size = 63, eq)]
pub(crate) struct Meta {
    #[ribbit(size = 59)]
    pub(crate) key: key::Array,
    pub(crate) frozen: bool,
    #[ribbit(size = 3)]
    pub(crate) kind: node::Kind,
}

impl Meta {
    pub(crate) fn match_or_insert(&self, key: &[u8]) -> Match {
        let key = key::Array::from_slice_len(key, self.key.len);

        if key == self.key {
            return Match::Full;
        }

        let prefix = key::Array::prefix(&key, &self.key);
        let (start, middle, end) = unsafe { self.key.expand(prefix) };
        Match::Partial { start, middle, end }
    }

    pub(crate) fn unfreeze(&self) -> Self {
        Self {
            frozen: false,
            ..*self
        }
    }
}

#[repr(transparent)]
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
#[ribbit::pack(size = 64)]
pub(crate) struct Data(u64);

impl Data {
    pub(crate) fn new_node<N, I>(edges: I) -> Self
    where
        N: node::Info,
        I: IntoIterator<Item = (u8, Meta, Data)>,
    {
        let mut node = Box::new(N::default());

        for (key, meta, data) in edges {
            let edge = node.reserve(key).expect("Node can fit all edges");
            edge.set_low(meta);
            edge.set_high(data);
        }

        let node = Box::leak(node) as *mut N;
        Self(node as u64)
    }

    pub(crate) fn new_leaf(leaf: u64) -> Self {
        Self(leaf)
    }

    pub(crate) fn to_leaf(self) -> u64 {
        self.0
    }

    pub(crate) unsafe fn to_node<'a>(self, kind: node::Kind) -> Option<Or<u64, node::Ref<'a>>> {
        match kind {
            node::Kind::None => None,
            node::Kind::Leaf => Some(Or::L(self.0)),
            node::Kind::Node3 => (self.0 as *mut Node3)
                .as_ref()
                .map(node::Ref::Node3)
                .map(Or::R),
            node::Kind::Node15 => (self.0 as *mut Node15)
                .as_ref()
                .map(node::Ref::Node15)
                .map(Or::R),
            node::Kind::Node256 => (self.0 as *mut Node256)
                .as_ref()
                .map(node::Ref::Node256)
                .map(Or::R),
        }
    }

    pub(crate) unsafe fn deallocate(self, kind: node::Kind) {
        match kind {
            node::Kind::None | node::Kind::Leaf => {
                unreachable!()
            }
            node::Kind::Node3 => drop(Box::from_raw(self.0 as *mut Node3)),
            node::Kind::Node15 => drop(Box::from_raw(self.0 as *mut Node15)),
            node::Kind::Node256 => drop(Box::from_raw(self.0 as *mut Node256)),
        }
    }
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

#[derive(Debug)]
pub(crate) enum Match {
    Full,
    Partial {
        start: key::Array,
        middle: u8,
        end: key::Array,
    },
}
