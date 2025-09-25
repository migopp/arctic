use ribbit::u3;

use crate::byte;

#[derive(Copy, Clone, Debug, Default)]
pub struct Iter {
    buffer: u64,
    len: u8,
}

impl Iter {
    #[inline]
    pub(super) fn new(buffer: u64, len: u8) -> Self {
        validate!(len <= 8);
        validate_eq!(buffer.unbounded_shr((len as u32) << 3), 0);
        Self { buffer, len }
    }
}

impl byte::Iterator for Iter {
    #[inline]
    fn len(&self) -> usize {
        self.len as usize
    }

    #[inline]
    fn peek(&self, len: u3) -> ribbit::Packed<byte::Array> {
        validate!(len.value() as usize <= self.len());

        byte::Array::from_u64_truncate(self.buffer, len)
    }

    #[inline]
    fn take(&mut self, len: u3) -> ribbit::Packed<byte::Array> {
        validate!(len.value() as usize <= self.len());

        let array = byte::Array::from_u64_truncate(self.buffer, len);
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
}

impl byte::Stack for Iter {
    #[inline]
    fn push_array(&mut self, array: ribbit::Packed<byte::Array>) {
        validate!(self.len + array.len().value() <= 8);
        self.buffer |= array.buffer().value() << (self.len << 3);
        self.len += array.len().value();
    }

    #[inline]
    fn push_byte(&mut self, byte: u8) {
        validate!(self.len < 8);
        self.buffer |= (byte as u64) << (self.len << 3);
        self.len += 1;
    }

    #[inline]
    fn pop(&mut self, count: usize) {
        validate!(self.len as usize >= count);
        self.len -= count as u8;
        self.buffer &= (1u64 << (self.len << 3)) - 1;
    }
}

macro_rules! impl_from {
    ($($from:ty: $len:expr),* $(,)?) => {
        $(
            impl From<$from> for Iter {
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
        )*
    };
}

impl_from!(
    u8: 1,
    u16: 2,
    u32: 4,
    u64: 8,
);
