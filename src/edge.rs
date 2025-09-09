use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;
use ribbit::u48;

use crate::key;
use crate::node;
use crate::node::Node256;
use crate::node::Node3;

#[derive(Copy, Clone, Debug, Default)]
#[ribbit::pack(size = 128, debug)]
pub(crate) struct Edge {
    #[ribbit(size = 72)]
    pub(crate) key: key::Array,

    pub(crate) frozen: bool,

    #[ribbit(size = 2)]
    pub(crate) kind: node::Kind,

    #[ribbit(offset = 80)]
    pub(crate) next: u48,
}

impl Edge {
    pub(crate) fn r#match(&self, key: &[u8]) -> Match {
        if cfg!(feature = "opt-empty-match") && key.is_empty() {
            return Match::Full {
                len: key::Len::ZERO,
                child: self.child(),
            };
        }

        let search_key = key::Array::from_slice(key);
        let edge_key = self.key;
        let prefix_len = key::Array::prefix(&search_key, &edge_key);

        // Fast path: successful traversal
        if search_key.len >= edge_key.len && edge_key.len == prefix_len {
            return Match::Full {
                len: prefix_len,
                child: self.child(),
            };
        }

        assert!(
            search_key.len >= edge_key.len || edge_key.len != prefix_len,
            "Precondition: no key is a prefix of another key",
        );

        let (start, middle, end) = edge_key.expand(prefix_len);
        Match::Partial { start, middle, end }
    }

    pub(crate) fn freeze(edge: &Atomic128<Self>) {
        let mut old = edge.load(Ordering::Relaxed);

        while !old.frozen {
            match edge.compare_exchange(
                old,
                Self {
                    frozen: true,
                    ..old
                },
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(conflict) => old = conflict,
            }
        }
    }

    pub(crate) fn leaf(&self) -> Option<u48> {
        match self.kind {
            node::Kind::None => None,
            node::Kind::Leaf => Some(self.next),
            _ => unreachable!(),
        }
    }

    pub(crate) fn child(&self) -> Option<Child> {
        let leaf = self.next;
        let pointer = leaf.value();

        match self.kind {
            node::Kind::None => None,
            node::Kind::Leaf => Some(Child::Leaf),
            node::Kind::Node3 => Some(Child::Node(node::Ref::Node3(pointer as *mut Node3))),
            node::Kind::Node256 => Some(Child::Node(node::Ref::Node256(pointer as *mut Node256))),
        }
    }

    pub(crate) unsafe fn deallocate(self) {
        let pointer = self.next.value();
        match self.kind {
            node::Kind::None | node::Kind::Leaf => {
                unreachable!()
            }
            node::Kind::Node3 => drop(Box::from_raw(pointer as *mut Node3)),
            node::Kind::Node256 => drop(Box::from_raw(pointer as *mut Node256)),
        }
    }
}

#[derive(Debug)]
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
pub(crate) enum Child {
    Leaf,
    Node(node::Ref),
}

#[derive(Debug)]
pub(crate) enum Match {
    Full {
        len: key::Len,
        child: Option<Child>,
    },
    Partial {
        start: key::Array,
        middle: u8,
        end: key::Array,
    },
}
