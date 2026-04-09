use core::ops::BitAnd as _;
use core::ops::BitOr as _;
use core::ops::Not as _;

use ribbit::traits::Integer as _;
use ribbit::u3;
use ribbit::u56;

pub trait Prefix: Send + Sync + ribbit::Unpack<Loose = u64> {
    const HAZARD_NULL: Self;
    const HAZARD_ROOT: Self;

    fn into_prefix(self, value: bool, bits: Option<usize>) -> Self;

    fn is_active(self) -> bool;

    fn is_conflict(self, other: Self) -> bool;
    fn is_conflict_simd(self, other: [Self; 4]) -> bool;

    fn bytes(&self) -> usize;

    fn is_node(self) -> bool;

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
#[ribbit(size = 64, packed(rename = "BePacked"), debug)]
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
    len: u3,

    #[ribbit(offset = 8)]
    prefix: u56,
}

impl Be {
    #[inline]
    pub(crate) fn new_hazard(prefix: u64, bits: usize) -> ribbit::Packed<Self> {
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
                Self::extract(prefix, bits) | bits as u64 | 0b0000_0111,
            )
        }
    }

    // Mask off everything except top `bits`
    #[inline]
    fn extract(prefix: u64, bits: usize) -> u64 {
        validate_eq!(bits & 0b111, 0);
        validate!((bits >> 3) <= u3::MAX.value() as usize);

        prefix & !(u64::MAX >> bits)
    }
}

impl Prefix for BePacked {
    const HAZARD_NULL: Self = Self::new(false, false, false, u3::new(0), u56::new(0));
    const HAZARD_ROOT: Self = Self::new(true, true, true, u3::new(0), u56::new(0));

    #[inline]
    fn into_prefix(self, value: bool, bits: Option<usize>) -> Self {
        match bits {
            Some(bits) if bits < (self.len().value() as usize) << 3 => unsafe {
                let prefix = Be::extract(self.value, bits);
                Self::new_unchecked(prefix | bits as u64)
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
    fn is_conflict_simd(self, prefix: [Self; 4]) -> bool {
        use core::arch::x86_64::*;
        validate!(self.is_active());
        validate!(self.node() ^ self.value());

        unsafe {
            // set hazard and broadcast prefix
            let h = _mm256_set_epi64x(
                prefix[3].value as i64,
                prefix[2].value as i64,
                prefix[1].value as i64,
                prefix[0].value as i64,
            );
            let p = _mm256_set1_epi64x(self.value as i64);

            let zeros = _mm256_setzero_si256();
            let ones = _mm256_set1_epi64x(-1i64);

            // Case: `hazard` doesn't protect node or value
            // (h & p) & 0b11
            let type_bits = _mm256_and_si256(_mm256_and_si256(h, p), _mm256_set1_epi64x(0b11));

            // != 0
            let type_match = _mm256_xor_si256(_mm256_cmpeq_epi64(type_bits, zeros), ones);

            // Case: `hazard` protects prefixes only, and `prefix` is higher up the tree
            // get overlap bit
            let h_no_overlap =
                _mm256_cmpeq_epi64(_mm256_and_si256(h, _mm256_set1_epi64x(0b100)), zeros);

            // fetch len
            let h_len = _mm256_and_si256(_mm256_srli_epi64::<3>(h), _mm256_set1_epi64x(0b111));
            let p_len = _mm256_and_si256(_mm256_srli_epi64::<3>(p), _mm256_set1_epi64x(0b111));

            // !h.overlap() && h.len > p.len
            let skip = _mm256_and_si256(h_no_overlap, _mm256_cmpgt_epi64(h_len, p_len));

            // Case: Overlapping prefix
            // h ^ p
            let xor = _mm256_xor_si256(h, p);

            // min_len * 8
            let min_len = _mm256_min_epu16(h_len, p_len);
            let bits = _mm256_slli_epi64::<3>(min_len);

            // Be::extract logic
            let shift = _mm256_sub_epi64(_mm256_set1_epi64x(64), bits);
            let prefix_mask = _mm256_sllv_epi64(ones, shift);

            // (xor & mask) == 0
            let overlap = _mm256_cmpeq_epi64(_mm256_and_si256(xor, prefix_mask), zeros);

            // combine, type_match & !skip & overlap
            let result = _mm256_and_si256(
                type_match,
                _mm256_and_si256(_mm256_andnot_si256(skip, ones), overlap),
            );
            _mm256_testz_si256(result, result) == 0
        }
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
                .bitand(u56::from(u8::MAX).not())
                .bitor(u56::from(age)),
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
#[ribbit(size = 64, packed(rename = "LePacked"), debug)]
pub struct Le {
    prefix: u56,

    pub(super) node: bool,
    pub(super) value: bool,
    pub(super) overlap: bool,

    len: u3,
}

impl Le {
    #[inline]
    pub(crate) fn new_hazard(prefix: u64, bits: usize) -> ribbit::Packed<Self> {
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
                Self::extract(prefix, bits) | const { 0b111u64 << 56 } | ((bits as u64) << 56),
            )
        }
    }

    // Mask off everything except bottom `bits`
    #[inline]
    fn extract(prefix: u64, bits: usize) -> u64 {
        validate_eq!(bits & 0b111, 0);
        validate!((bits >> 3) <= u3::MAX.value() as usize);

        prefix & ((1u64 << bits) - 1)
    }
}

impl Prefix for LePacked {
    const HAZARD_NULL: Self = Self::new(u56::new(0), false, false, false, u3::new(0));
    const HAZARD_ROOT: Self = Self::new(u56::new(0), true, true, true, u3::new(0));

    #[inline]
    fn into_prefix(self, value: bool, bits: Option<usize>) -> Self {
        match bits {
            Some(bits) if bits < (self.len().value() as usize) << 3 => {
                let prefix = Le::extract(self.value, bits);
                Self::new(
                    unsafe { u56::new_unchecked(prefix) },
                    !value,
                    value,
                    false,
                    u3::new((bits >> 3) as u8),
                )
            }
            Some(_) | None => self.with_node(!value).with_value(value),
        }
    }

    #[inline]
    fn is_active(self) -> bool {
        // Protects either values or nodes
        self.value & const { 0b11u64 << 56 } > 0
    }

    #[inline]
    fn is_conflict(self, prefix: Self) -> bool {
        validate!(self.is_active());
        validate!(prefix.node() ^ prefix.value());

        // Case: `hazard` doesn't protect node or value
        if (self.value & prefix.value) & const { 0b11u64 << 56 } == 0 {
            return false;
        }

        // Case: `hazard` protects prefixes only, and `prefix` is higher up the tree
        if !self.overlap() && self.len() > prefix.len() {
            return false;
        }

        self.is_overlap(prefix)
    }

    fn is_conflict_simd(self, prefix: [Self; 4]) -> bool {
        use core::arch::x86_64::*;
        validate!(self.node() ^ self.value());
        validate!(self.is_active());

        unsafe {
            // set hazard and broadcast prefix
            let h = _mm256_set_epi64x(
                prefix[3].value as i64,
                prefix[2].value as i64,
                prefix[1].value as i64,
                prefix[0].value as i64,
            );
            let p = _mm256_set1_epi64x(self.value as i64);

            let zeros = _mm256_setzero_si256();
            let ones = _mm256_set1_epi64x(-1i64);

            // Case: `hazard` doesn't protect node or value
            // (h & p) & (0b11 << 56)
            let type_bits =
                _mm256_and_si256(_mm256_and_si256(h, p), _mm256_set1_epi64x(0b11 << 56));
            // != 0
            let type_match = _mm256_xor_si256(_mm256_cmpeq_epi64(type_bits, zeros), ones);

            // Case: `hazard` protects prefixes only, and `prefix` is higher up the tree
            // get overlap bit
            let h_no_overlap =
                _mm256_cmpeq_epi64(_mm256_and_si256(h, _mm256_set1_epi64x(0b100 << 56)), zeros);

            // fetch len
            let h_len = _mm256_and_si256(_mm256_srli_epi64::<59>(h), _mm256_set1_epi64x(0b111));
            let p_len = _mm256_and_si256(_mm256_srli_epi64::<59>(p), _mm256_set1_epi64x(0b111));

            // !h.overlap() && h.len > p.len
            let skip = _mm256_and_si256(h_no_overlap, _mm256_cmpgt_epi64(h_len, p_len));

            // Case: Overlapping prefix
            // h ^ p
            let xor = _mm256_xor_si256(h, p);

            // min_len * 8
            let min_len = _mm256_min_epu16(h_len, p_len);
            let bits = _mm256_slli_epi64::<3>(min_len);

            // Be::extract logic
            let one = _mm256_set1_epi64x(1);
            let prefix_mask = _mm256_sub_epi64(_mm256_sllv_epi64(one, bits), one);

            let overlap = _mm256_cmpeq_epi64(_mm256_and_si256(xor, prefix_mask), zeros);

            // combine: type_match & !skip & overlap
            let result = _mm256_and_si256(
                type_match,
                _mm256_and_si256(_mm256_andnot_si256(skip, ones), overlap),
            );
            _mm256_testz_si256(result, result) == 0
        }
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
        (self.prefix().value() >> 48) as u8
    }

    /// For measurement purposes only
    #[inline]
    fn with_age(self, age: u8) -> Self {
        self.with_prefix(
            self.prefix()
                .bitand(const { u56::new(0xFFu64 << 48) })
                .bitor(u56::new((age as u64) << 48)),
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

#[cfg(test)]
mod tests {
    use super::{Be, BePacked, Le, LePacked, Prefix};

    #[test]
    fn be_simd_vs_scalar() {
        let cases: Vec<(BePacked, [BePacked; 4])> = vec![
            // prefix=value, mix of overlapping + disjoint hazards
            (
                be_p(0xABCD_0000_0000_0000, 16, true),
                [
                    be(0xABCD_0000_0000_0000, 16),
                    be(0x1234_0000_0000_0000, 16),
                    be(0xAB00_0000_0000_0000, 8),
                    BePacked::HAZARD_NULL,
                ],
            ),
            // all disjoint hazards
            (
                be_p(0xAA00_0000_0000_0000, 8, true),
                [
                    be(0xBB00_0000_0000_0000, 8),
                    be(0xCC00_0000_0000_0000, 8),
                    be(0xDD00_0000_0000_0000, 8),
                    BePacked::HAZARD_NULL,
                ],
            ),
            // null padding
            (
                be_p(0xABCD_0000_0000_0000, 16, true),
                [
                    be(0xFF00_0000_0000_0000, 8),
                    BePacked::HAZARD_NULL,
                    BePacked::HAZARD_NULL,
                    BePacked::HAZARD_NULL,
                ],
            ),
            // depth skip: hazard deeper, no overlap bit
            (
                be_p(0xABCD_0000_0000_0000, 16, false),
                [
                    be(0xABCD_EF00_0000_0000, 24).without_overlap(),
                    BePacked::HAZARD_NULL,
                    BePacked::HAZARD_NULL,
                    BePacked::HAZARD_NULL,
                ],
            ),
            // conflict only in last lane
            (
                be_p(0xABCD_0000_0000_0000, 16, true),
                [
                    be(0xFF00_0000_0000_0000, 8),
                    be(0xEE00_0000_0000_0000, 8),
                    be(0xDD00_0000_0000_0000, 8),
                    be(0xABCD_0000_0000_0000, 16),
                ],
            ),
            // root hazard overlaps everything
            (
                be_p(0xABCD_0000_0000_0000, 16, true),
                [
                    be(0, 0),
                    BePacked::HAZARD_NULL,
                    BePacked::HAZARD_NULL,
                    BePacked::HAZARD_NULL,
                ],
            ),
            // node prefix instead of value
            (
                be_p(0xABCD_0000_0000_0000, 16, false),
                [
                    be(0xABCD_0000_0000_0000, 16),
                    BePacked::HAZARD_NULL,
                    BePacked::HAZARD_NULL,
                    BePacked::HAZARD_NULL,
                ],
            ),
            (
                be_p(0xABCD_0000_0000_0000, 16, true),
                [
                    be(0xABCD_0000_0000_0000, 16).without_node(),
                    BePacked::HAZARD_NULL,
                    BePacked::HAZARD_NULL,
                    BePacked::HAZARD_NULL,
                ],
            ),
            // hazard protects only values, prefix is a node → no type match
            (
                be_p(0xABCD_0000_0000_0000, 16, false),
                [
                    be(0xABCD_0000_0000_0000, 16).without_node(),
                    BePacked::HAZARD_NULL,
                    BePacked::HAZARD_NULL,
                    BePacked::HAZARD_NULL,
                ],
            ),
            // all 4 lanes conflict
            (
                be_p(0xAB00_0000_0000_0000, 8, true),
                [
                    be(0xAB00_0000_0000_0000, 8),
                    be(0xABCD_0000_0000_0000, 16),
                    be(0xABCD_EF00_0000_0000, 24),
                    be(0, 0),
                ],
            ),
            // prefix shorter than hazard, same path → conflict
            (
                be_p(0xAB00_0000_0000_0000, 8, true),
                [
                    be(0xABCD_0000_0000_0000, 16),
                    BePacked::HAZARD_NULL,
                    BePacked::HAZARD_NULL,
                    BePacked::HAZARD_NULL,
                ],
            ),
            // prefix longer than hazard, same path → conflict
            (
                be_p(0xABCD_EF00_0000_0000, 24, true),
                [
                    be(0xAB00_0000_0000_0000, 8),
                    BePacked::HAZARD_NULL,
                    BePacked::HAZARD_NULL,
                    BePacked::HAZARD_NULL,
                ],
            ),
            // single bit difference in prefix
            (
                be_p(0xABCE_0000_0000_0000, 16, true),
                [
                    be(0xABCD_0000_0000_0000, 16),
                    BePacked::HAZARD_NULL,
                    BePacked::HAZARD_NULL,
                    BePacked::HAZARD_NULL,
                ],
            ),
            // depth skip WITH overlap bit → should NOT skip
            (
                be_p(0xABCD_0000_0000_0000, 16, false),
                [
                    be(0xABCD_EF00_0000_0000, 24),
                    BePacked::HAZARD_NULL,
                    BePacked::HAZARD_NULL,
                    BePacked::HAZARD_NULL,
                ],
            ),
            // // max length prefix
            // (
            //     be_p(0xABCD_EF01_2345_6700, 56),
            //     [
            //         be(0xABCD_EF01_2345_6700, 56),
            //         BePacked::HAZARD_NULL,
            //         BePacked::HAZARD_NULL,
            //         BePacked::HAZARD_NULL,
            //     ],
            // ),
        ];

        for (i, (prefix, hazards)) in cases.iter().enumerate() {
            let scalar = hazards
                .iter()
                .any(|h| h.is_active() && h.is_conflict(*prefix));
            let simd = prefix.is_conflict_simd(*hazards);
            assert_eq!(scalar, simd, "Be case {i}");
        }
    }

    #[test]
    fn le_simd_vs_scalar() {
        let cases: Vec<(LePacked, [LePacked; 4])> = vec![
            (
                le_p(0xABCD, 16, true),
                [
                    le(0xABCD, 16),
                    le(0x1234, 16),
                    le(0xAB, 8),
                    LePacked::HAZARD_NULL,
                ],
            ),
            (
                le_p(0xAA, 8, true),
                [le(0xBB, 8), le(0xCC, 8), le(0xDD, 8), LePacked::HAZARD_NULL],
            ),
            (
                le_p(0xABCD, 16, true),
                [
                    le(0xFF, 8),
                    LePacked::HAZARD_NULL,
                    LePacked::HAZARD_NULL,
                    LePacked::HAZARD_NULL,
                ],
            ),
            (
                le_p(0xABCD, 16, false),
                [
                    le(0xABCDEF, 24).without_overlap(),
                    LePacked::HAZARD_NULL,
                    LePacked::HAZARD_NULL,
                    LePacked::HAZARD_NULL,
                ],
            ),
            (
                le_p(0xABCD, 16, true),
                [le(0xFF, 8), le(0xEE, 8), le(0xDD, 8), le(0xABCD, 16)],
            ),
            (
                le_p(0xABCD, 16, true),
                [
                    le(0, 0),
                    LePacked::HAZARD_NULL,
                    LePacked::HAZARD_NULL,
                    LePacked::HAZARD_NULL,
                ],
            ),
            (
                le_p(0xABCD, 16, true),
                [
                    le(0xABCD, 16).without_node(),
                    LePacked::HAZARD_NULL,
                    LePacked::HAZARD_NULL,
                    LePacked::HAZARD_NULL,
                ],
            ),
            (
                le_p(0xABCD, 16, false),
                [
                    le(0xABCD, 16).without_node(),
                    LePacked::HAZARD_NULL,
                    LePacked::HAZARD_NULL,
                    LePacked::HAZARD_NULL,
                ],
            ),
            (
                le_p(0xAB, 8, true),
                [le(0xAB, 8), le(0xABCD, 16), le(0xABCDEF, 24), le(0, 0)],
            ),
            (
                le_p(0xAB, 8, true),
                [
                    le(0xABCD, 16),
                    LePacked::HAZARD_NULL,
                    LePacked::HAZARD_NULL,
                    LePacked::HAZARD_NULL,
                ],
            ),
            (
                le_p(0xABCDEF, 24, true),
                [
                    le(0xAB, 8),
                    LePacked::HAZARD_NULL,
                    LePacked::HAZARD_NULL,
                    LePacked::HAZARD_NULL,
                ],
            ),
            (
                le_p(0xABCE, 16, true),
                [
                    le(0xABCD, 16),
                    LePacked::HAZARD_NULL,
                    LePacked::HAZARD_NULL,
                    LePacked::HAZARD_NULL,
                ],
            ),
            (
                le_p(0xABCD, 16, false),
                [
                    le(0xABCDEF, 24),
                    LePacked::HAZARD_NULL,
                    LePacked::HAZARD_NULL,
                    LePacked::HAZARD_NULL,
                ],
            ),
            (
                le_p(0xABCDEF012345, 48, true),
                [
                    le(0xABCDEF012345, 48),
                    LePacked::HAZARD_NULL,
                    LePacked::HAZARD_NULL,
                    LePacked::HAZARD_NULL,
                ],
            ),
        ];

        for (i, (prefix, hazards)) in cases.iter().enumerate() {
            let scalar = hazards
                .iter()
                .any(|h| h.is_active() && h.is_conflict(*prefix));
            let simd = prefix.is_conflict_simd(*hazards);
            assert_eq!(scalar, simd, "Le case {i}");
        }
    }

    fn be(prefix: u64, bits: usize) -> BePacked {
        Be::new_hazard(prefix, bits)
    }
    fn be_p(prefix: u64, bits: usize, val: bool) -> BePacked {
        be(prefix, bits).into_prefix(val, Some(bits))
    }
    fn le(prefix: u64, bits: usize) -> LePacked {
        Le::new_hazard(prefix, bits)
    }
    fn le_p(prefix: u64, bits: usize, val: bool) -> LePacked {
        le(prefix, bits).into_prefix(val, Some(bits))
    }
}
