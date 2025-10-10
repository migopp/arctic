use core::fmt;
use core::ops::BitOr;
use core::ops::BitXor;

use ribbit::u6;

use crate::key;

/// Immutable fixed-size array of up to 7 bytes.
#[derive(Copy, Clone, Default, PartialEq, Eq)]
pub(crate) struct Array(u64);

impl Array {
    pub(crate) const EMPTY: Self = Array(0);
    pub(crate) const MASK_LEN: u64 = 0b0011_1111;
    pub(crate) const MASK_DATA: u64 = 0xFFFF_FFFF_FFFF_FF00;
    pub(crate) const MASK: u64 = Self::MASK_LEN | Self::MASK_DATA;

    #[inline]
    pub(crate) const fn from_u64_truncate(value: u64, len: Len) -> Self {
        unsafe { Self::new_unchecked(value & len.mask() | len.bits() as u64) }
    }

    #[inline]
    pub(crate) const fn new_masked(value: u64) -> Self {
        unsafe { Self::new_unchecked(value & Self::MASK) }
    }

    #[inline]
    pub(crate) const unsafe fn new_unchecked(value: u64) -> Self {
        const fn invariant(value: u64) -> bool {
            let bytes = value.to_be_bytes();
            let mut i = (value & Array::MASK_LEN) as usize;
            while i < 7 {
                if bytes[i] != 0 {
                    return false;
                }
                i += 1;
            }
            true
        }

        validate!(value & Self::MASK == value);
        validate!(invariant(value));
        Self(value)
    }

    #[inline]
    pub(crate) const fn value(self) -> u64 {
        self.0
    }

    #[inline]
    pub(crate) const fn len(self) -> Len {
        Len(unsafe { u6::new_unchecked(self.0 as u8) })
    }

    #[inline]
    #[cfg_attr(not(feature = "smr-hazard"), expect(dead_code))]
    pub(crate) fn is_overlapping(self, other: Self) -> bool {
        let len = self.len().min(other.len());
        self.equal_up_to(other, len)
    }

    #[inline]
    pub(crate) fn match_prefix<K: key::Read>(self, key: &mut K) -> Option<MatchPrefix> {
        let len = self.len().min_bits(key.bits());
        let key = key.take(len);
        if self == key {
            Some(MatchPrefix::Full(len))
        } else if self.equal_up_to(key, len) {
            Some(MatchPrefix::Partial)
        } else {
            None
        }
    }

    #[inline]
    pub(crate) fn match_exact<K: key::Read>(self, key: &mut K) -> Option<Len> {
        let len = self.len().min_bits(key.bits());
        (key.take(len) == self).then_some(len)
    }

    #[inline]
    pub(crate) fn match_split<K: key::Read>(self, key: &mut K) -> MatchSplit {
        let len = self.len().min_bits(key.bits());
        let key = key.take(len);

        if key == self {
            return MatchSplit::Full(len);
        }

        let len_prefix = unsafe {
            Len::from_bits_unchecked(
                key.0
                    .bitxor(self.0)
                    .bitor(1u64.rotate_right(1) >> len.bits())
                    .leading_zeros() as u8
                    & !0b111u8,
            )
        };

        let shift = len_prefix.bits() as u32 + 8;
        MatchSplit::Partial {
            start: Self::from_u64_truncate(self.0, len_prefix),
            middle: self.0.rotate_left(shift) as u8,
            end: Self::from_u64_truncate(self.0 << shift, unsafe {
                Len::from_bits_unchecked(self.len().bits() - len_prefix.bits() - 8)
            }),
        }
    }

    pub(crate) fn compress(self, byte: u8, child: Self) -> Option<Self> {
        let parent_bits = self.len().bits();
        let child_bits = child.len().bits();
        let len = Len::from_bits(parent_bits + Len::ONE.bits() + child_bits)?;
        let shift = parent_bits as u32 + 8;
        Some(Self::from_u64_truncate(
            self.0
                .bitor((byte as u64).rotate_right(shift))
                .bitor(child.0 >> shift),
            len,
        ))
    }

    #[inline]
    fn equal_up_to(self, other: Self, len: Len) -> bool {
        validate!(self.len() >= len);
        validate!(other.len() >= len);
        (self.0 ^ other.0) & len.mask() == 0
    }

    #[cfg(test)]
    pub(crate) fn with_bytes<F: FnOnce(&[u8]) -> T, T>(self, with: F) -> T {
        let bytes = self.0.to_be_bytes();
        let slice = &bytes[..self.len().bytes() as usize];
        with(slice)
    }
}

#[derive(Debug)]
pub(crate) enum MatchPrefix {
    Full(Len),
    Partial,
}

#[derive(Debug)]
pub(crate) enum MatchSplit {
    Full(Len),
    Partial {
        start: Array,
        middle: u8,
        end: Array,
    },
}

impl IntoIterator for Array {
    type Item = u8;
    type IntoIter = core::iter::Take<core::array::IntoIter<u8, 8>>;
    fn into_iter(self) -> Self::IntoIter {
        self.0
            .to_be_bytes()
            .into_iter()
            .take(self.len().bytes() as usize)
    }
}

impl fmt::Debug for Array {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(*self).finish()
    }
}

/// Length in bits.
#[repr(transparent)]
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Len(u6);

impl Len {
    const ONE: Self = Self::from_bits(8).unwrap();
    pub(crate) const MAX: Self = Self::from_bits(56).unwrap();

    #[inline]
    pub(crate) const fn from_bits(bits: u8) -> Option<Self> {
        validate!(bits & 0b111 == 0);
        if bits <= 56 {
            Some(Self(u6::new(bits)))
        } else {
            None
        }
    }

    #[inline]
    #[cfg(test)]
    pub(crate) const fn from_bytes(bytes: u8) -> Option<Self> {
        Self::from_bits(bytes << 3)
    }

    #[inline]
    pub(crate) const unsafe fn from_bits_unchecked(len: u8) -> Self {
        validate!(len & 0b111 == 0);
        validate!(len <= 56);
        Self(u6::new_unchecked(len))
    }

    #[inline]
    pub(crate) fn min_bits(self, bits: usize) -> Self {
        validate_eq!(bits & 0b111, 0);
        unsafe { Len::from_bits_unchecked((self.0.value() as usize).min(bits) as u8) }
    }

    #[inline]
    pub(crate) const fn bits(self) -> u8 {
        self.0.value()
    }

    #[inline]
    pub(crate) const fn bytes(self) -> u8 {
        self.0.value() >> 3
    }

    #[inline]
    const fn mask(self) -> u64 {
        !(u64::MAX >> self.bits())
    }

    #[inline]
    fn min(self, other: Self) -> Self {
        Self(self.0.min(other.0))
    }
}
