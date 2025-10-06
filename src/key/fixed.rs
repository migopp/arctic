use core::fmt;
use core::ops::BitOr as _;

use crate::byte;
use crate::key;

#[derive(Copy, Clone, Default, PartialEq, Eq)]
pub struct Reader {
    buffer: u64,
    len: u8,
}

impl Reader {
    #[inline]
    pub(super) fn new(buffer: u64, len: u8) -> Self {
        validate!(len <= 64);
        validate_eq!(len & 0b111, 0);
        Self { buffer, len }
    }

    #[inline]
    pub(super) fn with_bytes<F: FnOnce(&[u8]) -> T, T>(&self, with: F) -> T {
        with(&self.buffer.to_be_bytes()[..(self.len as usize) >> 3])
    }
}

impl key::Read for Reader {
    #[inline]
    fn len(&self) -> usize {
        self.len as usize
    }

    #[inline]
    fn peek(&self, len: ribbit::Packed<byte::Len>) -> ribbit::Packed<byte::Array> {
        validate!(len.bits() as usize <= self.len());

        let buffer = self.buffer.rotate_left(len.bits() as u32);
        ribbit::Packed::<byte::Array>::from_u64_truncate(buffer, len)
    }

    #[inline]
    fn take(&mut self, len: ribbit::Packed<byte::Len>) -> ribbit::Packed<byte::Array> {
        validate!(len.bits() as usize <= self.len());
        self.buffer = self.buffer.rotate_left(len.bits() as u32);
        self.len -= len.bits();
        ribbit::Packed::<byte::Array>::from_u64_truncate(self.buffer, len)
    }

    #[inline]
    fn next(&mut self) -> Option<u8> {
        let some = self.len > 0;
        self.buffer = self.buffer.rotate_left(8);
        self.len = self.len.saturating_sub(8);
        some.then_some(self.buffer as u8)
    }

    #[inline]
    fn prefix(&self, other: &Self) -> Self {
        let max = self.len.min(other.len);

        let prefix = (self.buffer ^ other.buffer)
            .bitor(0x8000_0000_0000_0000u64.unbounded_shr(max as u32))
            .leading_zeros()
            & !0b111;

        Self {
            buffer: self.buffer,
            len: prefix as u8,
        }
    }
}

impl fmt::Debug for Reader {
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
        let len = array.len().bits();
        validate!(self.len + len <= 64);
        self.buffer <<= len;
        self.buffer |= array.buffer().value();
        self.len += len;
    }

    #[inline]
    fn push(&mut self, byte: u8) {
        validate!(self.len < 64);
        self.buffer <<= 8;
        self.buffer |= byte as u64;
        self.len += 8;
    }

    #[inline]
    fn truncate(&mut self, len: usize) {
        validate!(self.len as usize >= len);
        validate!(len <= 64);
        self.buffer >>= self.len as usize - len;
        self.len = len as u8;
    }
}

impl From<Reader> for Writer {
    fn from(fixed: Reader) -> Self {
        Self {
            buffer: fixed.buffer.unbounded_shr(64u32 - (fixed.len as u32)),
            len: fixed.len,
        }
    }
}

macro_rules! impl_unsigned_int {
    ($($from:ty: $len:expr),* $(,)?) => {
        $(
            impl From<$from> for Reader {
                #[inline]
                fn from(value: $from) -> Self {
                    let len = $len << 3;
                    Self {
                        buffer: (value as u64) << (64 - len),
                        len,
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
                #[inline]
                fn partial_cmp(&self, value: &$from) -> Option<core::cmp::Ordering> {
                    <$from>::from(*self).partial_cmp(&value)
                }
            }

            impl PartialEq<$from> for Writer {
                #[inline]
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

    fn take_all<N, I: IntoIterator<Item = u8>>(initial: N, lens: I)
    where
        fixed::Reader: From<N>,
    {
        let mut iter = fixed::Reader::from(initial);
        let initial = iter.with_bytes(|bytes| bytes.to_vec());

        let mut index = 0;
        for len in lens
            .into_iter()
            .map(byte::Len::from_bytes)
            .map(Option::unwrap)
        {
            assert_eq!(iter.len() >> 3, initial.len() - index);
            ribbit::Packed::<byte::Array>::with_bytes(iter.take(len), |a| {
                assert_eq!(a, &initial[index..][..len.bytes() as usize]);
            });
            index += len.bytes() as usize;
        }

        assert_eq!(iter.len() >> 3, initial.len() - index);
        if iter.len() > 0 {
            assert_eq!(iter.next(), Some(initial[index]));
        } else {
            assert_eq!(iter.next(), None);
        }
    }
}
