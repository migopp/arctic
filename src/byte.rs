use core::fmt;
use core::ops::BitOr;
use core::ops::Shl as _;
use core::ops::Shr as _;

use ribbit::u3;
use ribbit::u56;
use ribbit::Pack as _;

use crate::key;

/// Immutable fixed-size array of up to 7 bytes.
#[derive(Copy, Clone, Default, PartialEq, Eq, ribbit::Pack)]
#[ribbit(size = 59, packed(rename = ArrayPacked), eq)]
pub(crate) struct Array {
    #[ribbit(size = 56, get(vis = "pub(crate)"))]
    buffer: u56,

    #[ribbit(size = 3, get(rename = "len_internal"))]
    len: u3,
}

impl Array {
    pub(crate) const EMPTY: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(u56::new(0), u3::new(0));

    pub(crate) const MAX_LEN: u3 = u3::new(7);

    #[inline]
    pub(crate) fn min_len(left: usize, right: u3) -> u3 {
        // SAFETY: `left.min(right)` can be at most `right`, which is a valid u3
        unsafe { u3::new_unchecked(left.min(right.value() as usize) as u8) }
    }
}

impl ArrayPacked {
    #[inline]
    pub(crate) fn len(self) -> usize {
        self.len_internal().value() as usize
    }

    #[inline]
    pub(crate) fn slice(self, len: usize) -> Self {
        Self::from_u64_truncate(self.value.value(), Array::min_len(len, self.len_internal()))
    }

    #[inline]
    pub(crate) fn from_u64_truncate(value: u64, len: u3) -> Self {
        let mask = (1u64 << (len.value() << 3)) - 1;
        Self::new(unsafe { u56::new_unchecked(value & mask) }, len)
    }

    #[inline]
    #[cfg_attr(not(feature = "smr-hazard"), expect(dead_code))]
    pub(crate) fn has_prefix(self, prefix: Self) -> bool {
        match self.len().checked_sub(prefix.len()) {
            None => false,
            Some(0) => self == prefix,
            Some(len) => {
                //  7  6  5  4  3  2  1  0
                //           s' s' s' s  s   self, len = 5
                //                 p  p  p   prefix, len = 3
                //                 s' s' s'
                let diff = (self.value.value() >> (len << 3)) ^ prefix.value.value();
                let mask = (1u64 << (prefix.len() << 3)) - 1;
                diff & mask == 0
            }
        }
    }

    #[inline]
    pub(crate) fn match_prefix<K: key::Read>(self, key: &mut K) -> Option<u3> {
        let len = Array::min_len(key.len(), self.len_internal());
        (key.take(len) == self).then_some(len)
    }

    #[inline]
    pub(crate) fn match_split<K: key::Read>(self, key: &mut K) -> Match {
        let len = Array::min_len(key.len(), self.len_internal());
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
        let diff = (key.buffer().value() >> ((key.len() - len.value() as usize) << 3))
            ^ (self.buffer().value() >> ((self.len() - len.value() as usize) << 3));

        let len_start = diff.leading_zeros().shr(3u32) as u8 - (8 - len.value());
        let len_end = self.len() as u8 - len_start - 1;

        Match::Partial {
            start: unsafe {
                Self::new(
                    u56::new_unchecked(self.buffer().value() >> ((len_end + 1) << 3)),
                    u3::new_unchecked(len_start),
                )
            },
            middle: (self.value.value() >> (len_end << 3)) as u8,
            end: unsafe {
                Self::new(
                    u56::new_unchecked(self.value.value() & ((1u64 << (len_end << 3)) - 1)),
                    u3::new_unchecked(len_end),
                )
            },
        }
    }

    pub(crate) fn compress(self, byte: u8, child: Self) -> Option<Self> {
        let parent_len = self.len();
        let child_len = child.len();
        let len = parent_len + 1 + child_len;
        if len > ribbit::Unpacked::<Self>::MAX_LEN.value() as usize {
            return None;
        }

        let shift = child_len << 3;

        Some(Self::new(
            unsafe {
                u56::new_unchecked(
                    self.value
                        .value()
                        .shl(8 + shift)
                        .bitor((byte as u64) << shift)
                        .bitor(child.buffer().value()),
                )
            },
            unsafe { u3::new_unchecked(len as u8) },
        ))
    }

    #[cfg(test)]
    pub(crate) fn with_bytes<F: FnOnce(&[u8]) -> T, T>(self, with: F) -> T {
        let bytes = self.buffer().value().to_be_bytes();
        let slice = &bytes[8 - self.len()..];
        with(slice)
    }
}

#[derive(Debug)]
pub(crate) enum Match {
    Full(u3),
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
            .skip(8 - self.len())
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
