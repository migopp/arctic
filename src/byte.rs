use core::cmp;
use core::fmt::Debug;
use core::ops::BitAnd as _;
use core::ops::BitOr as _;
use core::ops::BitXor as _;
use core::ops::Shr as _;

use ribbit::u3;
use ribbit::u56;

use crate::key;

/// Immutable fixed-size array of up to 7 bytes.
#[derive(Copy, Clone, Default, PartialEq, Eq, ribbit::Pack)]
#[ribbit(size = 59, packed(rename = ArrayPacked), debug, eq)]
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
    const MASK: u64 = 0x00FF_FFFF_FFFF_FFFF;

    #[inline]
    pub(crate) fn len(self) -> usize {
        self.len_internal().value() as usize
    }

    #[inline]
    pub(crate) fn slice(self, len: usize) -> Self {
        Self::from_u64_truncate(self.value.value(), Array::min_len(len, self.len_internal()))
    }

    #[inline]
    pub(crate) fn from_u64_truncate(array: u64, len: u3) -> Self {
        let bit = (len.value() as u64) << 3;
        let mask = (1u64 << bit) - 1;
        // SAFETY: `len` <= 7, so `array & mask` must be within 56 bytes
        Self::new(unsafe { u56::new_unchecked(array & mask) }, len)
    }

    #[inline]
    #[cfg_attr(not(feature = "smr-hazard"), expect(dead_code))]
    pub(crate) fn has_prefix(self, prefix: Self) -> bool {
        match self.len_internal().cmp(&prefix.len_internal()) {
            cmp::Ordering::Less => false,
            cmp::Ordering::Equal => self == prefix,
            cmp::Ordering::Greater => {
                let bit = prefix.len() << 3;
                let mask = (1u64 << bit) - 1;
                (self.value.value() ^ prefix.value.value()) & mask == 0
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
        let edge_len = self.len_internal();
        let key_len = key.len();
        let len = Array::min_len(key_len, edge_len);

        let key = key.take(len);
        if key == self {
            return Match::Full(len);
        }

        let edge = self.value.value();
        let prefix_byte = key
            .value
            .value()
            .bitxor(edge)
            // Guarantee `trailing_zeros` cannot produce more than `len * 8` bits
            .bitor(1u64 << ((len.value() as u64) << 3))
            .trailing_zeros()
            .shr(3u32) as u8;

        let prefix_bit = (prefix_byte as u32) << 3;

        Match::Partial {
            start: unsafe {
                Self::new(
                    u56::new_unchecked(edge & ((1u64 << prefix_bit) - 1u64)),
                    u3::new_unchecked(prefix_byte),
                )
            },
            middle: (edge >> prefix_bit) as u8,
            end: unsafe {
                Self::new(
                    u56::new_unchecked((edge & Self::MASK) >> (prefix_bit + 8)),
                    u3::new_unchecked(edge_len.value() - prefix_byte - 1),
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

        let bit = parent_len << 3;
        Some(Self::new(
            unsafe {
                u56::new_unchecked(
                    self.value
                        .value()
                        .bitand(Self::MASK)
                        .bitor((byte as u64) << bit)
                        .bitor(child.value.value() << (bit + 8)),
                )
            },
            unsafe { u3::new_unchecked(len as u8) },
        ))
    }

    #[cfg(test)]
    pub(crate) fn with_bytes<F: FnOnce(&[u8]) -> T, T>(self, prefix: Option<u8>, with: F) -> T {
        let bytes = match prefix {
            // Implicitly shift off len
            Some(prefix) => ((self.value.value() << 8) | prefix as u64).to_ne_bytes(),
            None => self.buffer().value().to_ne_bytes(),
        };
        let slice = &bytes[..self.len() + prefix.is_some() as usize];
        with(slice)
    }

    pub(crate) fn bytes(&self) -> impl core::iter::Iterator<Item = u8> {
        self.value
            .value()
            .to_ne_bytes()
            .into_iter()
            .take(self.len())
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

impl Debug for Array {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let bytes = self
            .buffer
            .value()
            .to_ne_bytes()
            .into_iter()
            .take(self.len.value() as usize);

        f.debug_list().entries(bytes).finish()
    }
}
