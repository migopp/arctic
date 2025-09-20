use core::fmt::Debug;
use core::ops::BitAnd as _;
use core::ops::BitOr as _;
use core::ops::BitXor as _;
use core::ops::Shr as _;

use ribbit::u3;
use ribbit::u56;

#[derive(Copy, Clone, Default, PartialEq, Eq)]
#[ribbit::pack(size = 59, debug, eq)]
pub(crate) struct Array {
    #[ribbit(size = 56)]
    buffer: u56,

    #[ribbit(size = 3)]
    pub(crate) len: u3,
}

impl Array {
    pub(crate) const EMPTY: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(u56::new(0), u3::new(0));

    const MASK: u64 = 0x00FF_FFFF_FFFF_FFFF;
    pub(crate) const MAX: usize = 7;

    #[inline]
    pub(crate) fn from_slice<K: Iterator>(mut key: K) -> ribbit::Packed<Self> {
        let len = unsafe { u3::new_unchecked(key.len().min(Self::MAX) as u8) };
        key.take(len)
    }

    #[inline]
    pub(crate) fn match_prefix<K: Iterator>(key: &mut K, edge: ribbit::Packed<Self>) -> bool {
        let len = unsafe { u3::new_unchecked(key.len().min(edge.len().value() as usize) as u8) };
        key.take(len) == edge
    }

    #[inline]
    pub(crate) fn match_split<K: Iterator>(key: &mut K, edge: ribbit::Packed<Self>) -> Match {
        let edge_len = edge.len().value() as usize;
        let key_len = key.len();
        let len = unsafe { u3::new_unchecked(key_len.min(edge_len) as u8) };
        let key = key.take(len);
        if key == edge {
            return Match::Full;
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

        let prefix_bit = prefix_byte << 3;

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
                    u3::new_unchecked(edge_len as u8 - prefix_byte - 1),
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
        if len > Self::MAX {
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

    pub(crate) fn with_bytes<F: FnOnce(&[u8]) -> T, T>(&self, prefix: Option<u8>, with: F) -> T {
        let bytes = match prefix {
            Some(prefix) => (self.buffer.value() << 8 | prefix as u64).to_ne_bytes(),
            None => self.buffer.value().to_ne_bytes(),
        };
        let slice = &bytes[..self.len.value() as usize + prefix.is_some() as usize];
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
    Full,
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
    fn take(&mut self, len: u3) -> ribbit::Packed<Array>;
    fn next(&mut self) -> Option<u8>;
}

#[derive(Copy, Clone, Debug, Default)]
pub struct Fixed {
    buffer: u64,
    len: u8,
}

impl From<u8> for Fixed {
    fn from(value: u8) -> Self {
        Self {
            buffer: value as u64,
            len: 1,
        }
    }
}

impl From<u64> for Fixed {
    fn from(value: u64) -> Self {
        Self {
            buffer: if cfg!(target_endian = "little") {
                value.swap_bytes()
            } else {
                value
            },
            len: 8,
        }
    }
}

impl Iterator for Fixed {
    fn len(&self) -> usize {
        self.len as usize
    }

    fn take(&mut self, len: u3) -> ribbit::Packed<Array> {
        let bit = (len.value() as u64) << 3;
        let array = ribbit::Packed::<Array>::new(
            unsafe { u56::new_unchecked(self.buffer & ((1u64 << bit) - 1)) },
            len,
        );
        self.buffer >>= bit;
        self.len -= len.value();
        array
    }

    fn next(&mut self) -> Option<u8> {
        let some = self.len > 0;
        let byte = self.buffer as u8;
        self.buffer >>= 8;
        self.len = self.len.saturating_sub(1);
        some.then_some(byte)
    }
}
