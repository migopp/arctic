use core::cmp;
use core::fmt::Debug;
use core::ops::BitAnd as _;
use core::ops::BitOr as _;
use core::ops::BitXor as _;
use core::ops::Shr as _;

use ribbit::u3;
use ribbit::u56;

mod dynamic;
mod fixed;

pub(crate) use dynamic::Dynamic;
pub(crate) use fixed::Fixed;

#[derive(Copy, Clone, Default, PartialEq, Eq)]
#[ribbit::pack(size = 59, debug, eq)]
pub(crate) struct Array {
    #[ribbit(size = 56)]
    buffer: u56,

    #[ribbit(size = 3)]
    len: u3,
}

impl Array {
    pub(crate) const EMPTY: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(u56::new(0), u3::new(0));

    const MASK: u64 = 0x00FF_FFFF_FFFF_FFFF;
    pub(crate) const MAX_LEN: u3 = u3::new(7);

    #[inline]
    fn from_u64_truncate(array: u64, len: u3) -> ribbit::Packed<Self> {
        let bit = (len.value() as u64) << 3;
        let mask = (1u64 << bit) - 1;
        // SAFETY: `len` <= 7, so `array & mask` must be within 56 bytes
        ribbit::Packed::<Self>::new(unsafe { u56::new_unchecked(array & mask) }, len)
    }

    #[inline]
    pub(crate) fn has_prefix(key: ribbit::Packed<Self>, prefix: ribbit::Packed<Self>) -> bool {
        match key.len().cmp(&prefix.len()) {
            cmp::Ordering::Less => false,
            cmp::Ordering::Equal => key == prefix,
            cmp::Ordering::Greater => {
                let bit = (prefix.len().value() as u64) << 3;
                let mask = (1u64 << bit) - 1;
                (key.value.value() ^ prefix.value.value()) & mask == 0
            }
        }
    }

    #[inline]
    pub(crate) fn match_prefix<K: Iterator>(key: &mut K, edge: ribbit::Packed<Self>) -> Option<u3> {
        let len = Self::min_len(key.len(), edge.len());
        (key.take(len) == edge).then_some(len)
    }

    #[inline]
    pub(crate) fn match_split<K: Iterator>(key: &mut K, edge: ribbit::Packed<Self>) -> Match {
        let edge_len = edge.len();
        let key_len = key.len();
        let len = Self::min_len(key_len, edge_len);

        let key = key.take(len);
        if key == edge {
            return Match::Full(len);
        }

        let edge = edge.value.value();
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
                ribbit::Packed::<Self>::new(
                    u56::new_unchecked(edge & ((1u64 << prefix_bit) - 1u64)),
                    u3::new_unchecked(prefix_byte),
                )
            },
            middle: (edge >> prefix_bit) as u8,
            end: unsafe {
                ribbit::Packed::<Self>::new(
                    u56::new_unchecked((edge & Self::MASK) >> (prefix_bit + 8)),
                    u3::new_unchecked(edge_len.value() - prefix_byte - 1),
                )
            },
        }
    }

    pub(crate) fn compress(
        parent: ribbit::Packed<Self>,
        byte: u8,
        child: ribbit::Packed<Self>,
    ) -> Option<ribbit::Packed<Self>> {
        let parent_len = parent.len().value() as usize;
        let child_len = child.len().value() as usize;
        let len = parent_len + 1 + child_len;
        if len > Self::MAX_LEN.value() as usize {
            return None;
        }

        let bit = parent_len << 3;
        Some(ribbit::Packed::<Self>::new(
            unsafe {
                u56::new_unchecked(
                    parent
                        .value
                        .value()
                        .bitor((byte as u64) << bit)
                        .bitor(child.value.value() << (bit + 8))
                        .bitand(Self::MASK),
                )
            },
            unsafe { u3::new_unchecked(len as u8) },
        ))
    }

    #[inline]
    pub(crate) fn min_len(left: usize, right: u3) -> u3 {
        // SAFETY: `left.min(right)` can be at most `right`, which is a valid u3
        unsafe { u3::new_unchecked(left.min(right.value() as usize) as u8) }
    }

    #[cfg(test)]
    pub(crate) fn with_bytes<F: FnOnce(&[u8]) -> T, T>(
        key: ribbit::Packed<Self>,
        prefix: Option<u8>,
        with: F,
    ) -> T {
        let bytes = match prefix {
            // Implicitly shift off len
            Some(prefix) => ((key.value.value() << 8) | prefix as u64).to_ne_bytes(),
            None => key.buffer().value().to_ne_bytes(),
        };
        let slice = &bytes[..key.len().value() as usize + prefix.is_some() as usize];
        with(slice)
    }

    pub(crate) fn bytes(&self) -> impl core::iter::Iterator<Item = u8> {
        self.buffer
            .value()
            .to_ne_bytes()
            .into_iter()
            .take(self.len.value() as usize)
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
        f.debug_list().entries(self.bytes()).finish()
    }
}

pub(crate) trait Iterator: Clone + core::fmt::Debug + Default {
    fn len(&self) -> usize;

    fn peek(&self, len: u3) -> ribbit::Packed<Array>;

    #[inline]
    fn peek_all(&self) -> ribbit::Packed<Array> {
        self.peek(Array::min_len(self.len(), Array::MAX_LEN))
    }

    fn take(&mut self, len: u3) -> ribbit::Packed<Array>;
    fn next(&mut self) -> Option<u8>;
}
