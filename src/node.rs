use ribbit::atomic::A128;
use ribbit::u48;
use ribbit::unpack;

mod node256;

pub(crate) use node256::Node256;

pub(crate) trait Node {
    fn get(&self, key: &[u8]) -> Option<&A128<Slot>>;
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

impl Slot {
    pub(crate) fn traverse(&self, key: &mut &[u8]) -> Traverse {
        let search_len = key.len();
        let search_key = *key;

        let slot_len = self.len() as usize;
        let slot_key = self.key().to_be_bytes();

        assert!(slot_len <= 8);

        let match_len = slot_key
            .iter()
            .take(slot_len as usize)
            .zip(search_key)
            .take_while(|(slot_byte, search_byte)| slot_byte == search_byte)
            .count();

        *key = &key[match_len..];

        // Fast path: successful traversal
        if search_len >= slot_len && slot_len == match_len {
            return Traverse::Child(self.child());
        }

        assert!(
            search_len >= slot_len || slot_len != match_len,
            "Precondition: no key is a prefix of another key",
        );

        // Split out matching prefix
        let mut split = slot_key;
        split[match_len..].fill(0);
        Traverse::Split(u64::from_be_bytes(split))
    }

    pub(crate) fn child(&self) -> Option<Ref> {
        let pointer = self.next().value();

        match self.kind().unpack() {
            <unpack![Kind]>::Null => None,
            <unpack![Kind]>::Value => Some(Ref::Value(pointer as *mut u64 as *mut ())),
            <unpack![Kind]>::Node256 => Some(Ref::Node256(unsafe { &*(pointer as *mut Node256) })),
        }
    }
}

pub(crate) enum Ref<'a> {
    Value(*mut ()),
    Node256(&'a Node256),
}

#[ribbit::pack(size = 3)]
pub(crate) enum Kind {
    Null,
    Value,
    Node256,
}

pub(crate) enum Traverse<'a> {
    Child(Option<Ref<'a>>),
    Split(u64),
}
