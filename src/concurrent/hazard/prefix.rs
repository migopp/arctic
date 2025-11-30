use core::ops::BitAnd as _;
use core::ops::BitOr as _;
use core::ops::Not as _;

use ribbit::traits::Integer as _;
use ribbit::u120;
use ribbit::u4;

pub trait Prefix: Send + Sync {
    const HAZARD_NULL: Self;
    const HAZARD_ROOT: Self;

    fn into_prefix(self, value: bool, bits: Option<usize>) -> Self;

    #[expect(clippy::wrong_self_convention)]
    fn is_active(self) -> bool;

    #[expect(clippy::wrong_self_convention)]
    fn is_conflict(self, other: Self) -> bool;

    fn bytes(&self) -> usize;

    #[expect(clippy::wrong_self_convention)]
    fn is_node(self) -> bool;

    #[expect(clippy::wrong_self_convention)]
    fn is_value(self) -> bool;

    fn without_overlap(self) -> Self;
    fn without_node(self) -> Self;

    /// For measurement purposes only
    fn age(self) -> u8;

    /// For measurement purposes only
    fn with_age(self, age: u8) -> Self;
}

// NOTE: this type is used for both **hazards**, which guard
// parts of the tree, and prefixes of retired edges.
#[derive(Copy, Clone, Debug, ribbit::Pack)]
#[ribbit(size = 128, packed(rename = "BePacked"), debug)]
pub struct Be {
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
                Self::extract(prefix, bits) | bits as u128 | 0b0000_0111,
            )
        }
    }

    // Mask off everything except top `bits`
    #[inline]
    fn extract(prefix: u128, bits: usize) -> u128 {
        validate_eq!(bits & 0b111, 0);
        validate!((bits >> 3) <= u4::MAX.value() as usize);

        prefix & !(u128::MAX >> bits)
    }
}

impl Prefix for BePacked {
    const HAZARD_NULL: Self = Self::new(false, false, false, u4::new(0), u120::new(0));
    const HAZARD_ROOT: Self = Self::new(true, true, true, u4::new(0), u120::new(0));

    #[inline]
    fn into_prefix(self, value: bool, bits: Option<usize>) -> Self {
        match bits {
            Some(bits) if bits < (self.len().value() as usize) << 3 => unsafe {
                let prefix = Be::extract(self.value, bits);
                Self::new_unchecked(prefix | bits as u128)
            },
            Some(_) | None => self,
        }
        .with_node(!value)
        .with_value(value)
    }

    #[inline]
    fn is_active(self) -> bool {
        // Protects either values or nodes
        self.value & 0b11 > 0
    }

    #[inline]
    fn is_conflict(self, prefix: Self) -> bool {
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

    #[inline]
    fn is_node(self) -> bool {
        self.node()
    }

    #[inline]
    fn is_value(self) -> bool {
        self.value()
    }

    #[inline]
    fn without_node(self) -> Self {
        self.with_node(false)
    }

    #[inline]
    fn without_overlap(self) -> Self {
        self.with_overlap(false)
    }

    #[inline]
    fn bytes(&self) -> usize {
        self.len().value() as usize
    }

    /// For measurement purposes only
    #[inline]
    fn age(self) -> u8 {
        self.prefix().value() as u8
    }

    /// For measurement purposes only
    #[inline]
    fn with_age(self, age: u8) -> Self {
        self.with_prefix(
            self.prefix()
                .bitand(u120::from(u8::MAX).not())
                .bitor(u120::from(age)),
        )
    }
}

impl BePacked {
    #[inline]
    fn is_overlap(self, other: Self) -> bool {
        let len = self.len().min(other.len());
        let bits = (len.value() as usize) << 3;
        Be::extract(self.value ^ other.value, bits) == 0
    }
}

#[derive(Copy, Clone, Debug, ribbit::Pack)]
#[ribbit(size = 128, packed(rename = "LePacked"), debug)]
pub struct Le {
    prefix: u120,

    pub(super) node: bool,
    pub(super) value: bool,
    pub(super) overlap: bool,

    len: u4,
}

impl Le {
    #[inline]
    pub(crate) fn new_hazard(prefix: u128, bits: usize) -> ribbit::Packed<Self> {
        validate_eq!(bits & 0b111, 0);

        let bits = if cfg!(feature = "stat") {
            // Avoid clobbering logical age counter
            // Bits is > 0 (>= 8), since there can be no key with length 0
            bits - 8
        } else {
            bits
        };

        unsafe {
            ribbit::Packed::<Self>::new_unchecked(
                Self::extract(prefix, bits) | const { 0b111u128 << 120 } | ((bits as u128) << 120),
            )
        }
    }

    // Mask off everything except bottom `bits`
    #[inline]
    fn extract(prefix: u128, bits: usize) -> u128 {
        validate_eq!(bits & 0b111, 0);
        validate!((bits >> 3) <= u4::MAX.value() as usize);

        prefix & ((1u128 << bits) - 1)
    }
}

impl Prefix for LePacked {
    const HAZARD_NULL: Self = Self::new(u120::new(0), false, false, false, u4::new(0));
    const HAZARD_ROOT: Self = Self::new(u120::new(0), true, true, true, u4::new(0));

    #[inline]
    fn into_prefix(self, value: bool, bits: Option<usize>) -> Self {
        match bits {
            Some(bits) if bits < (self.len().value() as usize) << 3 => {
                let prefix = Le::extract(self.value, bits);
                Self::new(
                    unsafe { u120::new_unchecked(prefix) },
                    !value,
                    value,
                    false,
                    u4::new((bits >> 3) as u8),
                )
            }
            Some(_) | None => self.with_node(!value).with_value(value),
        }
    }

    #[inline]
    fn is_active(self) -> bool {
        // Protects either values or nodes
        self.value & const { 0b11u128 << 120 } > 0
    }

    #[inline]
    fn is_conflict(self, prefix: Self) -> bool {
        validate!(self.is_active());
        validate!(prefix.node() ^ prefix.value());

        // Case: `hazard` doesn't protect node or value
        if (self.value & prefix.value) & const { 0b11u128 << 120 } == 0 {
            return false;
        }

        // Case: `hazard` protects prefixes only, and `prefix` is higher up the tree
        if !self.overlap() && self.len() > prefix.len() {
            return false;
        }

        self.is_overlap(prefix)
    }

    #[inline]
    fn is_node(self) -> bool {
        self.node()
    }

    #[inline]
    fn is_value(self) -> bool {
        self.value()
    }

    #[inline]
    fn without_node(self) -> Self {
        self.with_node(false)
    }

    #[inline]
    fn without_overlap(self) -> Self {
        self.with_overlap(false)
    }

    #[inline]
    fn bytes(&self) -> usize {
        self.len().value() as usize
    }

    /// For measurement purposes only
    #[inline]
    fn age(self) -> u8 {
        (self.prefix().value() >> 112) as u8
    }

    /// For measurement purposes only
    #[inline]
    fn with_age(self, age: u8) -> Self {
        self.with_prefix(
            self.prefix()
                .bitand(const { u120::new(0xFFu128 << 112) })
                .bitor(u120::new((age as u128) << 112)),
        )
    }
}

impl LePacked {
    #[inline]
    fn is_overlap(self, other: Self) -> bool {
        let len = self.len().min(other.len());
        let bits = (len.value() as usize) << 3;
        Le::extract(self.value ^ other.value, bits) == 0
    }
}
