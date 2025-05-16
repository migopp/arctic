use ribbit::atomic::A128;
use ribbit::u48;
use ribbit::unpack;

mod node256;
mod node3;

use node256::Node256;
use node3::Node3;

pub(crate) trait Node {
    fn get(&self, key: u8) -> Option<&A128<Slot>>;

    fn get_or_reserve(&self, key: u8) -> Result<&A128<Slot>, ReserveError>;

    fn grow(&self, parent: &A128<Slot>) -> Result<(), GrowError>;

    fn help(&self, parent: &A128<Slot>, grow: bool) -> Result<(), ()>;
}

pub(crate) enum ReserveError {
    /// Encountered SMO operation in current node
    Freeze { grow: bool },

    /// Initiate grow SMO in current node
    Grow,
}

pub(crate) enum GrowError {
    /// Encountered SMO operation in parent
    Freeze { grow: bool },

    /// Reparent due to path expansion
    Expand,
}

#[ribbit::pack(size = 128)]
pub(crate) struct Slot {
    key: u64,
    len: u8,

    freeze: bool,

    #[ribbit(size = 3)]
    kind: Kind,
    grow: bool,

    #[ribbit(offset = 80)]
    next: u48,
}

impl Default for Slot {
    fn default() -> Self {
        Self::new(
            0,
            0,
            false,
            Kind::new(<unpack![Kind]>::Null),
            false,
            u48::new(0),
        )
    }
}

impl Slot {
    pub(crate) fn traverse(&self, key: &mut &[u8]) -> Traverse {
        let search_len = key.len();
        let search_key = *key;

        let slot_len = self.len() as usize;
        let slot_key = self.key().to_be_bytes();

        assert!(slot_len <= 8);

        let prefix_len = slot_key
            .iter()
            .take(slot_len)
            .zip(search_key)
            .take_while(|(slot_byte, search_byte)| slot_byte == search_byte)
            .count();

        *key = &key[prefix_len..];

        // Fast path: successful traversal
        if search_len >= slot_len && slot_len == prefix_len {
            return Traverse::Walk(self.child());
        }

        assert!(
            search_len >= slot_len || slot_len != prefix_len,
            "Precondition: no key is a prefix of another key",
        );

        // Split out matching prefix
        let mut split = slot_key;
        split[prefix_len..].fill(0);
        Traverse::Split(u64::from_be_bytes(split))
    }

    fn child(&self) -> Tree {
        let pointer = self.next().value();

        match self.kind().unpack() {
            <unpack![Kind]>::Null => Tree::Leaf(None),
            <unpack![Kind]>::Value => Tree::Leaf(Some(pointer as *mut u64 as *mut ())),
            <unpack![Kind]>::Node3 => Tree::Node(Ref::Node3(pointer as *mut Node3)),
            <unpack![Kind]>::Node256 => Tree::Node(Ref::Node256(pointer as *mut Node256)),
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

#[ribbit::pack(size = 3)]
pub(crate) enum Kind {
    Null,
    Value,
    Node3,
    Node256,
}

pub(crate) enum Tree {
    Leaf(Option<*mut ()>),
    Node(Ref),
}

pub(crate) enum Traverse {
    Walk(Tree),
    Split(u64),
}
