use core::cmp::Ordering;

use ribbit::u120;
use ribbit::u4;

#[derive(Copy, Clone, ribbit::Pack)]
#[ribbit(size = 2, packed(rename = "KindPacked"), eq)]
pub(crate) enum Kind {
    /// Protect nothing
    Null = 0b00,

    /// Protect all nodes and values with keys that overlap this prefix
    Traversal = 0b01,

    /// Protect all nodes and values with keys underneath this prefix
    Prefix = 0b11,

    /// Protect all values underneath this prefix
    Value = 0b10,
}

impl KindPacked {
    const NULL: Self = Self::new_null();

    fn is_null(self) -> bool {
        self == Self::NULL
    }

    fn is_prefix(self) -> bool {
        self.value.value() & 0b10 > 0
    }

    fn is_node(self) -> bool {
        self.value.value() & 0b01 > 0
    }
}

#[derive(Copy, Clone, ribbit::Pack)]
#[ribbit(size = 128, packed(rename = "BePacked"))]
pub(crate) struct Be {
    #[ribbit(size = 2)]
    kind: Kind,
    len: u4,
    #[ribbit(offset = 8)]
    prefix: u120,
}

impl BePacked {
    // Clear lowest 8 bits
    const MASK_PREFIX: u128 = u128::MAX << 8;
    const DEFAULT: Self = Self::new(ribbit::Packed::<Kind>::new_null(), u4::new(0), u120::new(0));

    fn is_prefix(self, other: Self) -> bool {
        match self.len().cmp(&other.len()) {
            Ordering::Less | Ordering::Equal => self.is_overlap(other),
            Ordering::Greater => false,
        }
    }

    fn is_overlap(self, other: Self) -> bool {
        let len = self.len().min(other.len());
        let mask = !(Self::MASK_PREFIX >> ((len.value() as u128) << 3));
        (self.value ^ other.value) & mask == 0
    }
}
