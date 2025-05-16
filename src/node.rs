use ribbit::u48;

mod node256;

pub(crate) use node256::Node256;

pub(crate) trait Node {
    fn get(&self, key: &[u8]) -> Option<Match>;
}

#[ribbit::pack(size = 128)]
pub(crate) struct Slot {
    key: u64,
    len: u8,

    freeze: bool,
    value: bool,
    #[ribbit(size = 2)]
    kind: Kind,
    grow: bool,

    #[ribbit(offset = 80)]
    next: u48,
}

#[ribbit::pack(size = 2)]
pub(crate) enum Kind {
    N4,
    N256,
}

pub(crate) enum Match<'a> {
    Full {
        slot: &'a Slot,
    },

    Partial {
        slot: &'a Slot,
        prefix: u8,
        suffix: u8,
    },
}
