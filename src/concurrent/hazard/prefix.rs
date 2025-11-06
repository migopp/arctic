use ribbit::traits::Integer as _;
use ribbit::u120;
use ribbit::u4;

// NOTE: this type is used for both **hazards**, which guard
// parts of the tree, and prefixes of retired edges.
//
// For hazards, the 0th bit is whether nodes are protected,
// and the 1st bit is whether only prefixes are protected
// vs. all overlaps.
//
// For retired prefixes, the 0th bit is set for values.
#[derive(Copy, Clone, Debug, ribbit::Pack)]
#[ribbit(size = 2, packed(rename = "KindPacked"), debug, eq)]
pub(crate) enum Kind {
    /// Hazard: protect nothing
    /// Retired: node
    Null = 0b00,

    /// Hazard: protect all nodes and values with keys that overlap this prefix
    /// Retired: value
    Traversal = 0b01,

    /// Hazard: protect all nodes and values with keys underneath this prefix
    /// Retired: invalid
    Prefix = 0b11,

    /// Hazard: protect all values underneath this prefix
    /// Retired: invalid
    Value = 0b10,
}

impl Kind {
    pub(super) const HAZARD_NULL: ribbit::Packed<Self> = ribbit::Packed::<Self>::new_null();
    pub(super) const HAZARD_TRAVERSAL: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new_traversal();
    pub(super) const HAZARD_PREFIX: ribbit::Packed<Self> = ribbit::Packed::<Self>::new_prefix();
    pub(super) const HAZARD_VALUE: ribbit::Packed<Self> = ribbit::Packed::<Self>::new_value();

    pub(super) const RETIRED_NODE: ribbit::Packed<Self> = ribbit::Packed::<Self>::new_null();
    pub(super) const RETIRED_VALUE: ribbit::Packed<Self> = ribbit::Packed::<Self>::new_traversal();
}

impl KindPacked {
    pub(super) fn is_hazard_null(self) -> bool {
        self == Kind::HAZARD_NULL
    }
}

#[derive(Copy, Clone, Debug, ribbit::Pack)]
#[ribbit(size = 128, packed(rename = "BePacked"), debug)]
pub(crate) struct Be {
    #[ribbit(size = 2)]
    pub(super) kind: Kind,
    len: u4,
    #[ribbit(offset = 8)]
    prefix: u120,
}

impl Be {
    pub(super) const HAZARD_NULL: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(Kind::HAZARD_NULL, u4::new(0), u120::new(0));

    pub(crate) const HAZARD_ROOT: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(Kind::HAZARD_TRAVERSAL, u4::new(0), u120::new(0));

    #[inline]
    pub(crate) fn new_hazard(prefix: u128, bits: usize) -> ribbit::Packed<Self> {
        validate_eq!(bits & 0b111, 0);

        let bits = bits & 0b0111_1000;

        unsafe {
            ribbit::Packed::<Self>::new_unchecked(
                extract(prefix, bits)
                    | (bits >> 1) as u128
                    | Kind::HAZARD_TRAVERSAL.value.value() as u128,
            )
        }
    }
}

impl BePacked {
    pub(super) fn truncate(self, bits: usize) -> Self {
        if bits == 0 {
            return self;
        }

        validate!(bits <= ((self.len().value() as usize) << 3));

        let prefix = extract(self.value, bits);
        let len = (bits >> 1) as u128;

        // SAFETY: `extract` masks off metadata
        unsafe { Self::new_unchecked(prefix | len) }
    }

    pub(super) fn is_conflict(self, prefix: Self) -> bool {
        let hazard_kind = self.kind();
        let prefix_kind = prefix.kind();

        validate!(!hazard_kind.is_hazard_null());
        validate!(prefix_kind == Kind::RETIRED_NODE || prefix_kind == Kind::RETIRED_VALUE);

        let hazard_kind = hazard_kind.value.value();
        let prefix_kind = prefix_kind.value.value();

        // Case: `hazard` protects values only, and `prefix` is a node
        if (hazard_kind | prefix_kind) & 0b01 == 0 {
            return false;
        }

        // Case: `hazard` protects prefixes, and `prefix` is higher up the tree
        if (hazard_kind & 0b10 > 0) && self.len() > prefix.len() {
            return false;
        }

        self.is_overlap(prefix)
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
