use core::fmt;
use core::ops::BitOr as _;
use core::ops::Shr as _;

use ribbit::u3;

use crate::byte;
use crate::key;

#[derive(Copy, Clone, Default, PartialEq, Eq)]
pub struct Fixed {
    buffer: u64,
    len: u8,
}

impl Fixed {
    #[inline]
    pub(super) fn new(buffer: u64, len: u8) -> Self {
        validate!(len <= 8);
        Self { buffer, len }
    }

    #[inline]
    pub(super) fn with_bytes<F: FnOnce(&[u8]) -> T, T>(&self, with: F) -> T {
        with(&self.buffer.to_be_bytes()[..self.len as usize])
    }
}

impl key::Read for Fixed {
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
        self.buffer = self.buffer.unbounded_shl((len.value() as u32) << 3);
        self.len -= len.value();
        array
    }

    #[inline]
    fn next(&mut self) -> Option<u8> {
        let some = self.len > 0;
        let byte = (self.buffer >> 56) as u8;
        self.buffer <<= 8;
        self.len = self.len.saturating_sub(1);
        some.then_some(byte)
    }

    #[inline]
    fn prefix(&self, other: &Self) -> Self {
        let max = self.len.min(other.len);

        let prefix = (self.buffer ^ other.buffer)
            .bitor(0x8000_0000_0000_0000u64.unbounded_shr((max << 3) as u32))
            .leading_zeros()
            .shr(3);

        let mask = !(1u64.unbounded_shl(64u32 - (prefix << 3)).wrapping_sub(1));

        Self {
            buffer: self.buffer & mask,
            len: prefix as u8,
        }
    }
}

impl fmt::Debug for Fixed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.with_bytes(|bytes| bytes.fmt(f))
    }
}

#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
pub struct Writer {
    buffer: u64,
    len: u8,
}

impl key::Write for Writer {
    #[inline]
    fn len(&self) -> usize {
        self.len as usize
    }

    #[inline]
    fn extend(&mut self, array: ribbit::Packed<byte::Array>) {
        validate!(self.len + array.len() as u8 <= 8);
        self.buffer |= array.buffer().value();
        self.buffer = self.buffer.rotate_left(array.len() as u32);
        self.len += array.len() as u8;
    }

    #[inline]
    fn push(&mut self, byte: u8) {
        validate!(self.len < 8);
        self.buffer <<= 8;
        self.buffer |= byte as u64;
        self.len += 1;
    }

    #[inline]
    fn truncate(&mut self, len: usize) {
        validate!(self.len as usize >= len);
        validate!(len <= 8);
        self.buffer >>= (self.len as usize - len) << 3;
        self.len = len as u8;
    }
}

impl From<Fixed> for Writer {
    fn from(fixed: Fixed) -> Self {
        Self {
            buffer: fixed.buffer.unbounded_shr(64 - ((fixed.len as u32) << 3)),
            len: fixed.len,
        }
    }
}

macro_rules! impl_unsigned_int {
    ($($from:ty: $len:expr),* $(,)?) => {
        $(
            impl From<$from> for Fixed {
                #[inline]
                fn from(value: $from) -> Self {
                    Self {
                        buffer: (value as u64) << (64 - ($len << 3)),
                        len: $len,
                    }
                }
            }

            impl From<Writer> for $from {
                #[inline]
                fn from(writer: Writer) -> Self {
                    writer.buffer as $from
                }
            }

            impl PartialOrd<$from> for Writer {
                fn partial_cmp(&self, value: &$from) -> Option<core::cmp::Ordering> {
                    <$from>::from(*self).partial_cmp(&value)
                }
            }

            impl PartialEq<$from> for Writer {
                fn eq(&self, value: &$from) -> bool {
                    <$from>::from(*self) == *value
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

#[cfg(test)]
mod tests {
    use ribbit::u3;

    use crate::byte;
    use crate::key::fixed;
    use crate::key::Read as _;

    #[test]
    fn smoke() {
        take_all(0x1234_5678_9ABC_DEF0u64, [7, 1]);
    }

    #[test]
    fn take_0() {
        take_all(0x1234_5678_9ABC_DEF0u64, [0, 1, 0]);
    }

    fn take_all<N, I: IntoIterator<Item = usize>>(initial: N, lens: I)
    where
        fixed::Fixed: From<N>,
    {
        let mut iter = fixed::Fixed::from(initial);
        let initial = iter.with_bytes(|bytes| bytes.to_vec());

        let mut index = 0;
        for len in lens {
            assert_eq!(iter.len(), initial.len() - index);
            ribbit::Packed::<byte::Array>::with_bytes(iter.take(u3::new(len as u8)), |a| {
                assert_eq!(a, &initial[index..][..len]);
            });
            index += len;
        }

        assert_eq!(iter.len(), initial.len() - index);
        if iter.len() > 0 {
            assert_eq!(iter.next(), Some(initial[index]));
        } else {
            assert_eq!(iter.next(), None);
        }
    }
}
