use core::ops::BitOr as _;
use core::ops::Shr as _;

use ribbit::u3;

use crate::byte;
use crate::key;

#[derive(Copy, Clone, Debug, Default)]
pub struct Fixed {
    buffer: u64,
    len: u8,
}

impl Fixed {
    #[inline]
    pub(super) fn new(buffer: u64, len: u8) -> Self {
        validate!(len <= 8);
        validate_eq!(buffer.unbounded_shr((len as u32) << 3), 0);
        Self { buffer, len }
    }

    #[inline]
    pub(super) fn with_bytes<F: FnOnce(&[u8]) -> T, T>(&self, with: F) -> T {
        with(&self.buffer.to_ne_bytes()[..self.len as usize])
    }
}

impl key::Iterator for Fixed {
    #[inline]
    fn len(&self) -> usize {
        self.len as usize
    }

    #[inline]
    fn peek(&self, len: u3) -> ribbit::Packed<byte::Array> {
        validate!(len.value() as usize <= self.len());

        ribbit::Packed::<byte::Array>::from_u64_truncate(self.buffer, len)
    }

    #[inline]
    fn take(&mut self, len: u3) -> ribbit::Packed<byte::Array> {
        validate!(len.value() as usize <= self.len());

        let array = ribbit::Packed::<byte::Array>::from_u64_truncate(self.buffer, len);
        self.buffer >>= (len.value() as u64) << 3;
        self.len -= len.value();
        array
    }

    #[inline]
    fn next(&mut self) -> Option<u8> {
        let some = self.len > 0;
        let byte = self.buffer as u8;
        self.buffer >>= 8;
        self.len = self.len.saturating_sub(1);
        some.then_some(byte)
    }

    #[inline]
    fn prefix(&self, other: &Self) -> Self {
        let len = self.len.min(other.len);
        let prefix = (self.buffer ^ other.buffer)
            .bitor(1 << len)
            .trailing_zeros()
            .shr(3);
        let mask = 1u64.unbounded_shl(prefix << 3).wrapping_sub(1);
        Self {
            buffer: self.buffer & mask,
            len,
        }
    }
}

impl key::Stack for Fixed {
    #[inline]
    fn len(&self) -> usize {
        self.len as usize
    }

    #[inline]
    fn extend(&mut self, array: ribbit::Packed<byte::Array>) {
        validate!(self.len + array.len() as u8 <= 8);
        self.buffer |= array.buffer().value() << (self.len << 3);
        self.len += array.len() as u8;
    }

    #[inline]
    fn push(&mut self, byte: u8) {
        validate!(self.len < 8);
        self.buffer |= (byte as u64) << (self.len << 3);
        self.len += 1;
    }

    #[inline]
    fn truncate(&mut self, len: usize) {
        validate!(self.len as usize >= len);
        validate!(len <= 8);
        self.buffer &= 1u64.unbounded_shl((len << 3) as u32).wrapping_sub(1);
        self.len = len as u8;
    }
}

macro_rules! impl_unsigned_int {
    ($($from:ty: $len:expr),* $(,)?) => {
        $(
            impl From<$from> for Fixed {
                #[inline]
                fn from(value: $from) -> Self {
                    Self {
                        buffer: if cfg!(target_endian = "little") {
                            value.swap_bytes()
                        } else {
                            value
                        } as u64,
                        len: $len,
                    }
                }
            }

            impl From<&'_ Fixed> for $from {
                #[inline]
                fn from(iter: &Fixed) -> Self {
                    validate_eq!(iter.len, $len);
                    let value = (iter.buffer as $from);
                    if cfg!(target_endian = "little") {
                        value.swap_bytes()
                    } else {
                        value
                    }
                }
            }

            impl PartialEq<$from> for Fixed {
                fn eq(&self, value: &$from) -> bool {
                    <$from>::from(self) == *value
                }
            }
        )*
    };
}

impl_unsigned_int!(
    u8: 1,
    u16: 2,
    u32: 4,
    u64: 8,
);
