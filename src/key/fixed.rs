use core::fmt;
use core::ops::BitOr as _;

use crate::byte;
use crate::key;
use crate::key::Read as _;

#[derive(Copy, Clone, Default, PartialEq, Eq)]
pub struct Reader {
    buffer: u64,
    bits: u8,
}

impl Reader {
    #[inline]
    pub(super) fn new(buffer: u64, bits: u8) -> Self {
        validate!(bits <= 64);
        validate_eq!(bits & 0b111, 0);
        Self { buffer, bits }
    }

    #[inline]
    pub(super) fn with_bytes<F: FnOnce(&[u8]) -> T, T>(&self, with: F) -> T {
        with(&self.buffer.to_be_bytes()[..self.bytes()])
    }
}

impl key::Read for Reader {
    #[inline]
    fn bits(&self) -> usize {
        self.bits as usize
    }

    #[inline]
    fn peek(&self, len: byte::Len) -> byte::Array {
        validate!(len.bits() as usize <= self.bits());
        byte::Array::from_u64_truncate(self.buffer, len)
    }

    #[inline]
    fn take(&mut self, len: byte::Len) -> byte::Array {
        validate!(len.bits() as usize <= self.bits());
        let array = self.peek(len);
        self.buffer = self.buffer.unbounded_shl(len.bits() as u32);
        self.bits -= len.bits();
        array
    }

    #[inline]
    fn next(&mut self) -> Option<u8> {
        let byte = (self.bits > 0).then_some(self.buffer.rotate_left(8) as u8);
        self.buffer <<= 8;
        self.bits = self.bits.saturating_sub(8);
        byte
    }

    #[inline]
    fn prefix(&self, other: &Self) -> Self {
        let max = self.bits.min(other.bits);

        let prefix = (self.buffer ^ other.buffer)
            .bitor(0x8000_0000_0000_0000u64.unbounded_shr(max as u32))
            .leading_zeros()
            & !0b111;

        Self {
            buffer: self.buffer,
            bits: prefix as u8,
        }
    }

    #[inline]
    fn get(&self, bit: usize) -> u8 {
        self.buffer.rotate_left(8 + bit as u32) as u8
    }

    #[inline]
    fn slice(&self, bit: usize) -> Self {
        Self {
            buffer: self.buffer & !u64::MAX.unbounded_shr(bit as u32),
            bits: bit as u8,
        }
    }
}

impl fmt::Debug for Reader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.with_bytes(|bytes| bytes.fmt(f))
    }
}

#[derive(Copy, Clone, Default, PartialEq, Eq)]
pub struct Writer {
    buffer: u64,
    bits: u8,
}

impl key::Write for Writer {
    type Len = usize;

    #[inline]
    fn bits(&self) -> usize {
        self.bits as usize
    }

    #[inline]
    fn extend(&mut self, array: byte::Array) {
        validate!(self.bits + array.len().bits() <= 64);
        self.buffer |= (array.value() & !0xFF).unbounded_shr(self.bits as u32);
        self.bits += array.len().bits();
    }

    #[inline]
    fn push(&mut self, byte: u8) {
        validate!(self.bits < 64);
        self.buffer |= (byte as u64).rotate_right((8 + self.bits) as u32);
        self.bits += 8;
    }

    #[inline]
    fn truncate(&mut self, bits: usize) {
        validate!(self.bits as usize >= bits);
        validate!(bits <= 64);
        self.buffer &= !u64::MAX.unbounded_shr(bits as u32);
        self.bits = bits as u8;
    }
}

impl From<Reader> for Writer {
    #[inline]
    fn from(reader: Reader) -> Self {
        Self {
            buffer: reader.buffer & !u64::MAX.unbounded_shr(reader.bits as u32),
            bits: reader.bits,
        }
    }
}

impl core::fmt::Debug for Writer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let bytes = self.buffer.to_be_bytes();
        let len = self.bits as usize >> 3;
        f.debug_list().entries(bytes.into_iter().take(len)).finish()
    }
}

macro_rules! impl_unsigned_int {
    ($($from:ty: $len:expr),* $(,)?) => {
        $(
            impl From<$from> for Reader {
                #[inline]
                fn from(value: $from) -> Self {
                    let bits = $len << 3;
                    Self {
                        buffer: (value as u64) << (64 - bits),
                        bits,
                    }
                }
            }

            impl From<Writer> for $from {
                #[inline]
                fn from(writer: Writer) -> Self {
                    writer.buffer.rotate_left($len << 3) as $from
                }
            }
        )*
    };
}

impl PartialOrd<Reader> for Writer {
    #[inline]
    fn partial_cmp(&self, reader: &Reader) -> Option<core::cmp::Ordering> {
        validate_eq!(self.bits, reader.bits);
        self.buffer.partial_cmp(&reader.buffer)
    }
}

impl PartialEq<Reader> for Writer {
    #[inline]
    fn eq(&self, reader: &Reader) -> bool {
        validate_eq!(self.bits, reader.bits);
        self.buffer == reader.buffer
    }
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
            assert_eq!(iter.bytes(), initial.len() - index);
            byte::Array::with_bytes(iter.take(len), |a| {
                assert_eq!(a, &initial[index..][..len.bytes() as usize]);
            });
            index += len.bytes() as usize;
        }

        assert_eq!(iter.bytes(), initial.len() - index);
        if iter.bytes() > 0 {
            assert_eq!(iter.next(), Some(initial[index]));
        } else {
            assert_eq!(iter.next(), None);
        }
    }
}
