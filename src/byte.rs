use core::fmt;
use core::ops::BitOr;
use core::ops::Shl as _;

use ribbit::u56;
use ribbit::u6;
use ribbit::Pack as _;

use crate::key;

/// Immutable fixed-size array of up to 7 bytes.
#[derive(Copy, Clone, Default, PartialEq, Eq, ribbit::Pack)]
#[ribbit(size = 62, packed(rename = ArrayPacked), eq)]
pub(crate) struct Array {
    #[ribbit(size = 56, get(vis = "pub(crate)"))]
    buffer: u56,

    #[ribbit(size = 6, get(vis = "pub(crate)"))]
    len: Len,
}

impl Array {
    pub(crate) const EMPTY: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(u56::new(0), Len::ZERO);
}

impl ArrayPacked {
    #[inline]
    pub(crate) fn slice(self, len: usize) -> Self {
        Self::from_u64_truncate(self.value.value(), self.len().min(len))
    }

    #[inline]
    pub(crate) fn from_u64_truncate(value: u64, len: ribbit::Packed<Len>) -> Self {
        Self::new(unsafe { u56::new_unchecked(value & len.mask()) }, len)
    }

    #[inline]
    #[cfg_attr(not(feature = "smr-hazard"), expect(dead_code))]
    pub(crate) fn has_prefix(self, prefix: Self) -> bool {
        match self.len().bits().checked_sub(prefix.len().bits()) {
            None => false,
            Some(0) => self == prefix,
            Some(len) => {
                //  7  6  5  4  3  2  1  0
                //           s' s' s' s  s   self, len = 5
                //                 p  p  p   prefix, len = 3
                //                 s' s' s'
                let diff = (self.value.value() >> len) ^ prefix.value.value();
                diff & prefix.len().mask() == 0
            }
        }
    }

    #[inline]
    pub(crate) fn match_prefix<K: key::Read>(self, key: &mut K) -> Option<ribbit::Packed<Len>> {
        let len = self.len().min(key.len());
        (key.take(len) == self).then_some(len)
    }

    #[inline]
    pub(crate) fn match_split<K: key::Read>(self, key: &mut K) -> Match {
        let len = self.len().min(key.len());
        let key = key.take(len);

        if key == self {
            return Match::Full(len);
        }

        // 7 6 5 4 3 2 1 0
        //         s s x x  self, len = 32
        //     s s m e e e  key, len = 48
        //         s s m e  shift key for prefix
        //
        //             s s  start
        //               m  middle
        //           e e e  end
        let diff = (key.buffer().value() >> (key.len().bits() - len.bits()))
            ^ (self.buffer().value() >> (self.len().bits() - len.bits()));

        let len_start = (diff.leading_zeros() & !0b111) as u8 - (64 - len.bits());
        let len_end =
            unsafe { Len::from_bits_unchecked(self.len().bits() - len_start - Len::ONE.bits()) };

        Match::Partial {
            start: Self::from_u64_truncate(
                self.value.value() >> (len_end.bits() + Len::ONE.bits()),
                unsafe { Len::from_bits_unchecked(len_start) },
            ),
            middle: (self.value.value() >> len_end.bits()) as u8,
            end: Self::from_u64_truncate(self.value.value(), len_end),
        }
    }

    pub(crate) fn compress(self, byte: u8, child: Self) -> Option<Self> {
        let parent_bits = self.len().bits();
        let child_bits = child.len().bits();
        let len = Len::from_bits(parent_bits + Len::ONE.bits() + child_bits)?;
        Some(Self::from_u64_truncate(
            self.value
                .value()
                .shl(Len::ONE.bits() + child_bits)
                .bitor((byte as u64) << child_bits)
                .bitor(child.value.value()),
            len,
        ))
    }

    #[cfg(test)]
    pub(crate) fn with_bytes<F: FnOnce(&[u8]) -> T, T>(self, with: F) -> T {
        let bytes = self.buffer().value().to_be_bytes();
        let slice = &bytes[8 - self.len().bytes() as usize..];
        with(slice)
    }
}

#[derive(Debug)]
pub(crate) enum Match {
    Full(ribbit::Packed<Len>),
    Partial {
        start: ribbit::Packed<Array>,
        middle: u8,
        end: ribbit::Packed<Array>,
    },
}

impl IntoIterator for ArrayPacked {
    type Item = u8;
    type IntoIter = core::iter::Skip<core::array::IntoIter<u8, 8>>;
    fn into_iter(self) -> Self::IntoIter {
        self.value
            .value()
            .to_be_bytes()
            .into_iter()
            .skip(8 - self.len().bytes() as usize)
    }
}

impl fmt::Debug for Array {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.pack().fmt(f)
    }
}

impl fmt::Debug for ArrayPacked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(*self).finish()
    }
}

/// Length in bits.
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq, ribbit::Pack)]
#[ribbit(size = 6, packed(rename = LenPacked), debug, eq, ord)]
pub(crate) struct Len(u6);

impl Len {
    const ZERO: ribbit::Packed<Self> = Self::from_bits(0).unwrap();
    const ONE: ribbit::Packed<Self> = Self::from_bits(8).unwrap();
    pub(crate) const MAX: ribbit::Packed<Self> = Self::from_bits(56).unwrap();

    #[inline]
    pub(crate) const fn from_bits(bits: u8) -> Option<ribbit::Packed<Self>> {
        validate!(bits & 0b111 == 0);
        if bits <= 56 {
            Some(ribbit::Packed::<Self>::new(u6::new(bits)))
        } else {
            None
        }
    }

    #[inline]
    #[cfg(test)]
    pub(crate) const fn from_bytes(bytes: u8) -> Option<ribbit::Packed<Self>> {
        Self::from_bits(bytes << 3)
    }

    #[inline]
    pub(crate) const unsafe fn from_bits_unchecked(len: u8) -> ribbit::Packed<Self> {
        validate!(len & 0b111 == 0);
        validate!(len <= 56);
        ribbit::Packed::<Self>::new_unchecked(u6::new_unchecked(len))
    }
}

impl LenPacked {
    #[inline]
    pub(crate) fn min(self, other: usize) -> Self {
        unsafe { Len::from_bits_unchecked((self.value.value() as usize).min(other) as u8) }
    }

    #[inline]
    pub(crate) fn bits(self) -> u8 {
        self.value.value()
    }

    #[inline]
    pub(crate) fn bytes(self) -> u8 {
        self.value.value() >> 3
    }

    #[inline]
    fn mask(self) -> u64 {
        (1u64 << self.value.value()) - 1
    }
}
