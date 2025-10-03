use core::cmp;
use core::fmt;
use core::ops::BitOr as _;
use core::ops::BitXor as _;
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
        Self::from_u64_truncate(
            self.buffer().value() << 8,
            Array::min_len(len, self.len_internal()),
        )
    }

    #[inline]
    pub(crate) fn from_u64_truncate(value: u64, len: u3) -> Self {
        let mask = !(1u64
            .unbounded_shl((8 - len.value() as u32) << 3)
            .wrapping_sub(1));

        Self::new(unsafe { u56::new_unchecked((value & mask) >> 8) }, len)
    }

    #[inline]
    #[cfg_attr(not(feature = "smr-hazard"), expect(dead_code))]
    pub(crate) fn has_prefix(self, prefix: Self) -> bool {
        match self.len_internal().cmp(&prefix.len_internal()) {
            cmp::Ordering::Less => false,
            cmp::Ordering::Equal => self == prefix,
            cmp::Ordering::Greater => {
                // 7 6 5 4 3 2 1 0
                //   s s s s          self, len = 32
                //   p p p            prefix, len = 24
                // x x x              xor(self, prefix) << 8
                // 0 1 2 3 4 5 6 7
                //
                // shift_xor = 40
                // mask_xor = 0xFFFF_FF00_0000_0000
                let shift_xor = 64 - (prefix.len() << 3);
                let mask_xor = !(1u64.unbounded_shl(shift_xor as u32).wrapping_sub(1));
                ((self.value.value() ^ prefix.value.value()) << 8) & mask_xor == 0
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

        let edge = self.buffer().value();
        let prefix_byte = key.buffer().value().bitxor(edge).leading_zeros().shr(3u32) as u8;
        let prefix_bit = (prefix_byte as u32) << 3;

        // 7 6 5 4 3 2 1 0
        //   s s m e e e e
        // 0 1 2 3 4 5 6 7
        //       ^
        //    prefix = 24
        //
        // shift_middle = 32
        // mask_end     = 0x??00_0000_FFFF_FFFF
        // mask_start   = 0x??FF_FF00_0000_0000
        let shift_middle = 56 - prefix_bit;
        let mask_end = (1u64 << shift_middle) - 1;
        let mask_start = !mask_end << 8;

        Match::Partial {
            start: unsafe {
                Self::new(
                    u56::new_unchecked(edge & mask_start),
                    u3::new_unchecked(prefix_byte - 1),
                )
            },
            middle: (edge >> shift_middle) as u8,
            end: unsafe {
                Self::new(
                    u56::new_unchecked((edge & mask_end) << prefix_bit),
                    u3::new_unchecked(edge_len.value() - prefix_byte),
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

        // 7 6 5 4 3 2 1 0
        //   p p p p          parent_len = 32
        //               b
        //   c                child_len = 8
        //   p p p p b c
        // 0 1 2 3 4 5 6 7
        //
        // shift_byte = 16
        // shift_child = 40
        let shift_byte = 48 - (parent_len << 3);
        let shift_child = parent_len + 8;

        Some(Self::new(
            unsafe {
                u56::new_unchecked(
                    self.buffer()
                        .value()
                        .bitor((byte as u64) << shift_byte)
                        .bitor(child.buffer().value() >> shift_child),
                )
            },
            unsafe { u3::new_unchecked(len as u8) },
        ))
    }

    #[cfg(test)]
    pub(crate) fn with_bytes<F: FnOnce(&[u8]) -> T, T>(self, with: F) -> T {
        let bytes = self.buffer().value().to_be_bytes();
        let slice = &bytes[1..][..self.len()];
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
    type IntoIter = core::iter::Take<core::iter::Skip<core::array::IntoIter<u8, 8>>>;
    fn into_iter(self) -> Self::IntoIter {
        self.value
            .value()
            .to_be_bytes()
            .into_iter()
            .skip(1)
            .take(self.len())
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
