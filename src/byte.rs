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

    #[ribbit(size = 6, get(rename = "len_internal"))]
    len: Len,
}

/// Length in bits.
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq, ribbit::Pack)]
#[ribbit(size = 6, packed(rename = LenPacked), debug, eq)]
pub(crate) struct Len(u6);

impl Len {
    const ZERO: ribbit::Packed<Self> = ribbit::Packed::<Self>::new(u6::new(0));
}

impl LenPacked {
    #[inline]
    pub(crate) fn min(self, other: usize) -> Self {
        unsafe {
            Self::new_unchecked(u6::new_unchecked(
                (self.value.value() as usize).min(other) as u8
            ))
        }
    }

    #[inline]
    pub(crate) fn value(self) -> u8 {
        validate_eq!(self.value.value() & 0b111, 0);
        self.value.value()
    }
}

impl Array {
    pub(crate) const EMPTY: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(u56::new(0), Len::ZERO);

    pub(crate) const MAX_LEN: ribbit::Packed<Len> = ribbit::Packed::<Len>::new(u6::new(56));
}

impl ArrayPacked {
    #[inline]
    pub(crate) fn len(self) -> ribbit::Packed<Len> {
        self.len_internal()
    }

    #[inline]
    pub(crate) fn slice(self, len: usize) -> Self {
        Self::from_u64_truncate(self.value.value(), self.len_internal().min(len))
    }

    #[inline]
    pub(crate) fn from_u64_truncate(value: u64, len: ribbit::Packed<Len>) -> Self {
        let mask = (1u64 << len.value.value()) - 1;
        Self::new(unsafe { u56::new_unchecked(value & mask) }, len)
    }

    #[inline]
    #[cfg_attr(not(feature = "smr-hazard"), expect(dead_code))]
    pub(crate) fn has_prefix(self, prefix: Self) -> bool {
        match self.len().value().checked_sub(prefix.len().value()) {
            None => false,
            Some(0) => self == prefix,
            Some(len) => {
                //  7  6  5  4  3  2  1  0
                //           s' s' s' s  s   self, len = 5
                //                 p  p  p   prefix, len = 3
                //                 s' s' s'
                let diff = (self.value.value() >> len) ^ prefix.value.value();
                let mask = (1u64 << prefix.len().value()) - 1;
                diff & mask == 0
            }
        }
    }

    #[inline]
    pub(crate) fn match_prefix<K: key::Read>(self, key: &mut K) -> Option<ribbit::Packed<Len>> {
        let len = self.len_internal().min(key.len());
        (key.take(len) == self).then_some(len)
    }

    #[inline]
    pub(crate) fn match_split<K: key::Read>(self, key: &mut K) -> Match {
        let len = self.len_internal().min(key.len());
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
        let diff = (key.buffer().value() >> (key.len().value() - len.value()))
            ^ (self.buffer().value() >> (self.len().value() - len.value()));

        let len_start = (diff.leading_zeros() & !0b111) as u8 - (64 - len.value());
        let len_end = self.len().value() - len_start - 8;

        Match::Partial {
            start: unsafe {
                Self::new(
                    u56::new_unchecked(self.buffer().value() >> (len_end + 8)),
                    ribbit::Packed::<Len>::new_unchecked(u6::new_unchecked(len_start)),
                )
            },
            middle: (self.value.value() >> len_end) as u8,
            end: unsafe {
                Self::new(
                    u56::new_unchecked(self.value.value() & ((1u64 << len_end) - 1)),
                    ribbit::Packed::<Len>::new_unchecked(u6::new_unchecked(len_end)),
                )
            },
        }
    }

    pub(crate) fn compress(self, byte: u8, child: Self) -> Option<Self> {
        let parent_len = self.len().value();
        let child_len = child.len().value();
        let len = parent_len + 8 + child_len;
        if len > ribbit::Unpacked::<Self>::MAX_LEN.value() {
            return None;
        }

        Some(Self::new(
            unsafe {
                u56::new_unchecked(
                    self.value
                        .value()
                        .shl(8 + child_len)
                        .bitor((byte as u64) << child_len)
                        .bitor(child.buffer().value()),
                )
            },
            unsafe { ribbit::Packed::<Len>::new_unchecked(u6::new_unchecked(len)) },
        ))
    }

    #[cfg(test)]
    pub(crate) fn with_bytes<F: FnOnce(&[u8]) -> T, T>(self, with: F) -> T {
        let bytes = self.buffer().value().to_be_bytes();
        let slice = &bytes[((64 - self.len().value()) >> 3) as usize..];
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
            .skip((64 - self.len().value() as usize) >> 3)
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
