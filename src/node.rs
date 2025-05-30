use core::fmt::Debug;

use ribbit::atomic::A128;
use ribbit::u48;
use ribbit::unpack;

mod node256;
mod node3;

pub(crate) use node256::Node256;
pub(crate) use node3::Node3;

pub(crate) trait Node {
    fn get(&self, key: u8) -> Option<&A128<Slot>>;

    fn get_or_reserve(&self, key: u8) -> Result<&A128<Slot>, ReserveError>;

    fn reserve(&mut self, key: u8) -> Result<&mut A128<Slot>, ReserveError>;

    fn grow(&self, parent: &A128<Slot>) -> Result<Ref, GrowError>;

    fn help(&self, parent: &A128<Slot>, grow: bool) -> Result<(), ()>;
}

#[derive(Debug)]
pub(crate) enum ReserveError {
    /// Encountered SMO operation in current node
    Freeze { grow: bool },

    /// Initiate grow SMO in current node
    Grow,
}

#[derive(Debug)]
pub(crate) enum GrowError {
    /// Encountered SMO operation in parent
    Freeze { grow: bool },

    /// Reparent due to path expansion
    Expand,
}

#[ribbit::pack(size = 128, debug)]
pub(crate) struct Slot {
    pub(crate) key: u64,
    pub(crate) len: u8,

    pub(crate) freeze: bool,
    pub(crate) grow: bool,

    #[ribbit(size = 3)]
    pub(crate) kind: Kind,

    #[ribbit(offset = 80)]
    pub(crate) next: u48,
}

impl Default for Slot {
    fn default() -> Self {
        Self::new(
            0,
            0,
            false,
            false,
            Kind::new(<unpack![Kind]>::Uninit),
            u48::new(0),
        )
    }
}

impl Slot {
    pub(crate) fn traverse(&self, key: &[u8]) -> Traverse {
        let search_len = key.len();
        let search_key = key;

        let slot_len = self.len() as usize;
        let slot_key = self.key().to_be_bytes();

        assert!(slot_len <= 8);

        let prefix_len = slot_key
            .iter()
            .take(slot_len)
            .zip(search_key)
            .take_while(|(slot_byte, search_byte)| slot_byte == search_byte)
            .count();

        // Fast path: successful traversal
        if search_len >= slot_len && slot_len == prefix_len {
            return Traverse::Walk {
                len: prefix_len,
                child: self.child(),
            };
        }

        assert!(
            search_len >= slot_len || slot_len != prefix_len,
            "Precondition: no key is a prefix of another key",
        );

        let mut start = [0u8; 8];
        start[..prefix_len].copy_from_slice(&slot_key[..prefix_len]);

        let middle = slot_key[prefix_len];

        let mut end = [0u8; 8];
        end[..slot_len - prefix_len - 1].copy_from_slice(&slot_key[prefix_len + 1..slot_len]);

        Traverse::Split {
            start_len: prefix_len,
            end_len: slot_len - prefix_len - 1,
            start: u64::from_be_bytes(start),
            middle,
            end: u64::from_be_bytes(end),
        }
    }

    fn child(&self) -> Child {
        let leaf = self.next();
        let pointer = leaf.value();

        match self.kind().unpack() {
            <unpack![Kind]>::Uninit => Child::Uninit,
            <unpack![Kind]>::Invalid => Child::Leaf(None),
            <unpack![Kind]>::Valid => Child::Leaf(Some(leaf)),
            <unpack![Kind]>::Node3 => Child::Node(Ref::Node3(pointer as *mut Node3)),
            <unpack![Kind]>::Node256 => Child::Node(Ref::Node256(pointer as *mut Node256)),
        }
    }
}

pub(crate) enum Ref {
    Node3(*mut Node3),
    Node256(*mut Node256),
}

impl Ref {
    // FIXME: how to express lifetimes?
    pub(crate) unsafe fn as_node<'art>(&self) -> &'art dyn Node {
        match self {
            Ref::Node3(node) => unsafe { node.as_ref().unwrap() },
            Ref::Node256(node) => unsafe { node.as_ref().unwrap() },
        }
    }
}

impl Debug for Ref {
    fn fmt(&self, fmt: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Ref::Node3(node3) => unsafe { node3.as_ref().unwrap() }.fmt(fmt),
            Ref::Node256(node256) => unsafe { node256.as_ref().unwrap() }.fmt(fmt),
        }
    }
}

#[ribbit::pack(size = 3, debug)]
pub(crate) enum Kind {
    Uninit,
    Valid,
    Invalid,
    Node3,
    Node256,
}

#[derive(Debug)]
pub(crate) enum Child {
    Uninit,
    Leaf(Option<u48>),
    Node(Ref),
}

#[derive(Debug)]
pub(crate) enum Traverse {
    Walk {
        len: usize,
        child: Child,
    },
    Split {
        start_len: usize,
        end_len: usize,
        start: u64,
        middle: u8,
        end: u64,
    },
}
