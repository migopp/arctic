use core::ops::BitAnd as _;
use core::ops::BitOr as _;
use core::ops::Not as _;

use ribbit::traits::Integer as _;
use ribbit::u120;
use ribbit::u4;

// NOTE: this type is used for both **hazards**, which guard
// parts of the tree, and prefixes of retired edges.
#[derive(Copy, Clone, Debug, ribbit::Pack)]
#[ribbit(size = 128, packed(rename = "BePacked"), debug)]
pub(crate) struct Be {
    // Hazard: whether to protect nodes
    // Prefix: whether this is a node
    pub(super) node: bool,

    // Hazard: whether to protect values
    // Prefix: whether this is a value
    pub(super) value: bool,

    // Hazard: whether to protect overlaps (or just underneath prefix)
    // Prefix: ignore
    pub(super) overlap: bool,

    // NOTE: at offset 3 so we don't need to shift bits
    len: u4,

    #[ribbit(offset = 8)]
    prefix: u120,
}

impl Be {
    pub(super) const HAZARD_NULL: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(false, false, false, u4::new(0), u120::new(0));

    pub(crate) const HAZARD_ROOT: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(true, true, true, u4::new(0), u120::new(0));

    #[inline]
    pub(crate) fn new_hazard(prefix: u128, bits: usize) -> ribbit::Packed<Self> {
        validate_eq!(bits & 0b111, 0);

        let bits = bits & 0b0111_1000;

        let bits = if cfg!(feature = "stat") {
            // Avoid clobbering logical age counter
            // Bits is > 0 (>= 8), since there can be no key with length 0
            bits - 8
        } else {
            bits
        };

        unsafe {
            ribbit::Packed::<Self>::new_unchecked(
                // Protect nodes, values, and overlap
                extract(prefix, bits) | bits as u128 | 0b0000_0111,
            )
        }
    }
}

impl BePacked {
    /// Construct
    pub(super) fn into_prefix(self, value: bool, bits: Option<usize>) -> Self {
        match bits {
            Some(bits) if bits < (self.len().value() as usize) << 3 => unsafe {
                let prefix = extract(self.value, bits);
                Self::new_unchecked(prefix | bits as u128)
            },
            Some(_) | None => self,
        }
        .with_node(!value)
        .with_value(value)
    }

    pub(super) fn is_active(self) -> bool {
        // Protects either values or nodes
        self.value & 0b11 > 0
    }

    pub(super) fn is_conflict(self, prefix: Self) -> bool {
        validate!(self.is_active());
        validate!(prefix.node() ^ prefix.value());

        // Case: `hazard` doesn't protect node or value
        if (self.value & prefix.value) & 0b11 == 0 {
            return false;
        }

        // Case: `hazard` protects prefixes only, and `prefix` is higher up the tree
        if !self.overlap() && self.len() > prefix.len() {
            return false;
        }

        self.is_overlap(prefix)
    }

    pub(super) fn bytes(&self) -> usize {
        self.len().value() as usize
    }

    /// For measurement purposes only
    pub(super) fn age(self) -> u8 {
        self.prefix().value() as u8
    }

    /// For measurement purposes only
    pub(super) fn with_age(self, age: u8) -> Self {
        self.with_prefix(
            self.prefix()
                .bitand(u120::from(u8::MAX).not())
                .bitor(u120::from(age)),
        )
    }

    fn is_overlap(self, other: Self) -> bool {
        let len = self.len().min(other.len());
        let bits = (len.value() as usize) << 3;
        extract(self.value ^ other.value, bits) == 0
    }
}

// Mask off everything except top `bits`
fn extract(prefix: u128, bits: usize) -> u128 {
    validate_eq!(bits & 0b111, 0);
    validate!((bits >> 3) <= u4::MAX.value() as usize);

    prefix & !(u128::MAX >> bits)
}
