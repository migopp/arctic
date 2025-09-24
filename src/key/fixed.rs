use ribbit::u3;

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
}

impl key::Iterator for Fixed {
    #[inline]
    fn len(&self) -> usize {
        self.len as usize
    }

    #[inline]
    fn peek(&self, len: u3) -> ribbit::Packed<key::Array> {
        key::Array::from_u64_truncate(self.buffer, key::Array::min_len(self.len as usize, len))
    }

    #[inline]
    fn take(&mut self, len: u3) -> ribbit::Packed<key::Array> {
        let array = key::Array::from_u64_truncate(self.buffer, len);
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

macro_rules! impl_from {
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
        )*
    };
}

impl_from!(
    u8: 1,
    u16: 2,
    u32: 4,
    u64: 8,
);
