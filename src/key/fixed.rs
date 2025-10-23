use core::fmt;

use crate::byte;
use crate::key;
use crate::key::Read as _;

pub(super) trait Uint:
    'static
    + Sized
    + Copy
    + Default
    + fmt::Debug
    + Ord
    + Eq
    + core::ops::Shl<u8, Output = Self>
    + core::ops::ShlAssign<u8>
    + core::ops::Shr<u8, Output = Self>
    + core::ops::BitXor<Output = Self>
    + core::ops::BitOr<Output = Self>
    + core::ops::BitOrAssign
    + core::ops::Not<Output = Self>
    + core::ops::BitAnd<Output = Self>
{
    const MSB: Self;
    const MAX: Self;
    const BYTES: u8;
    const BITS: u8;

    fn with_be_bytes<F: FnOnce(&[u8]) -> T, T>(self, apply: F) -> T;

    fn most_significant_u64(self) -> u64;
    fn most_significant_u8(self) -> u8;

    #[inline]
    fn most_significant(self, bits: u8) -> Self {
        Self::MAX.unbounded_shr(bits).not().bitand(self)
    }

    fn unbounded_shr(self, bits: u8) -> Self;
    fn leading_zeros(self) -> u8;
    fn rotate_left(self, bits: u8) -> Self;

    fn from_most_significant_u64(value: u64) -> Self;
    fn from_u8(value: u8) -> Self;
}

#[derive(Copy, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Buffer<U> {
    buffer: U,
    bits: u8,
}

#[expect(private_bounds)]
impl<U: Uint> Buffer<U> {
    #[inline]
    pub fn new_masked(buffer: U, bits: u8) -> Self {
        unsafe {
            let bits = bits & !0b111;
            Self::new_unchecked(buffer.most_significant(bits), bits)
        }
    }

    #[inline]
    pub unsafe fn new_unchecked(buffer: U, bits: u8) -> Self {
        validate!(bits <= U::BITS);
        validate_eq!(bits & 0b111, 0);
        validate_eq!(buffer.most_significant(bits), buffer);
        Self { buffer, bits }
    }

    #[inline]
    pub(super) fn with_bytes<F: FnOnce(&[u8]) -> T, T>(&self, with: F) -> T {
        self.buffer
            .with_be_bytes(|bytes| with(&bytes[..self.bytes()]))
    }
}

impl<U: Uint> key::Read for Buffer<U> {
    #[inline]
    fn bits(&self) -> usize {
        self.bits as usize
    }

    #[inline]
    fn peek(&self, len: byte::Len) -> byte::Array {
        validate!(len.bits() as usize <= self.bits());
        validate_eq!(len.bits() & 0b111, 0);

        byte::Array::from_u64_truncate(self.buffer.most_significant_u64(), len)
    }

    #[inline]
    fn take(&mut self, len: byte::Len) -> byte::Array {
        validate!(len.bits() as usize <= self.bits());
        validate_eq!(len.bits() & 0b111, 0);

        if len.bits() == 0 {
            return byte::Array::EMPTY;
        }

        let array = self.peek(len);
        self.buffer <<= len.bits();
        self.bits -= len.bits();
        array
    }

    #[inline]
    fn next(&mut self) -> Option<u8> {
        if self.bits == 0 {
            return None;
        }

        let byte = self.buffer.most_significant_u8();
        self.buffer <<= 8;
        self.bits = self.bits.saturating_sub(8);
        Some(byte)
    }

    fn prefix(&self, other: &Self) -> Self {
        let max = self.bits.min(other.bits);
        let bits = (self.buffer ^ other.buffer).leading_zeros().min(max) & !0b111;
        Self {
            buffer: self.buffer.most_significant(bits),
            bits,
        }
    }

    #[inline]
    fn get(&self, bit: usize) -> u8 {
        validate!(bit <= U::BITS as usize - 8);

        self.buffer.rotate_left(bit as u8).most_significant_u8()
    }

    #[inline]
    fn slice(&self, bits: usize) -> Self {
        validate!(bits <= U::BITS as usize);

        let bits = bits as u8;
        Self {
            buffer: self.buffer.most_significant(bits),
            bits,
        }
    }
}

impl<U: Uint> key::Write for Buffer<U> {
    type Len = usize;

    #[inline]
    fn bits(&self) -> usize {
        self.bits as usize
    }

    #[inline]
    fn extend(&mut self, array: byte::Array) {
        validate!(self.bits + array.len().bits() <= U::BITS);

        if array.len().bits() == 0 {
            return;
        }

        self.buffer |= U::from_most_significant_u64(array.value() & !0xFF) >> self.bits;
        self.bits += array.len().bits();
    }

    #[inline]
    unsafe fn extend_nonempty_unchecked(&mut self, array: byte::Array) {
        validate!(self.bits + array.len().bits() <= U::BITS);
        validate!(self.bits >= 8);

        if array.len().bits() == 0 {
            return;
        }

        self.buffer |= U::from_most_significant_u64(array.value()) >> self.bits;
        self.bits += array.len().bits();
    }

    #[inline]
    fn push(&mut self, byte: u8) {
        validate!(self.bits <= U::BITS - 8);

        self.buffer |= U::from_u8(byte).shr(self.bits);
        self.bits += 8;
    }

    #[inline]
    fn truncate(&mut self, bits: usize) {
        validate!(self.bits as usize >= bits);
        validate!(bits <= U::BITS as usize);

        let bits = bits as u8;
        self.buffer = self.buffer.most_significant(bits);
        self.bits = bits;
    }
}

impl<U: Copy> From<&'_ Buffer<U>> for Buffer<U> {
    fn from(buffer: &'_ Buffer<U>) -> Self {
        *buffer
    }
}

impl<U: Uint> core::fmt::Debug for Buffer<U> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.with_bytes(|bytes| f.debug_list().entries(bytes).finish())
    }
}

macro_rules! impl_unsigned_int {
    ($($ty:ty: $bits:expr, $into:expr, $from:expr),* $(,)?) => {
        $(
            impl From<$ty> for Buffer<$ty> {
                #[inline]
                fn from(value: $ty) -> Self {
                    Self {
                        buffer: value,
                        bits: $bits,
                    }
                }
            }

            impl From<Buffer<$ty>> for $ty {
                #[inline]
                fn from(fixed: Buffer<$ty>) -> Self {
                    fixed.buffer
                }
            }

            impl Uint for $ty {
                const MSB: Self = (1 as $ty).rotate_right(1);
                const MAX: Self = <$ty>::MAX;
                const BYTES: u8 = (<$ty>::BITS as u8) >> 3;
                const BITS: u8 = <$ty>::BITS as u8;

                #[inline]
                fn with_be_bytes<F: FnOnce(&[u8]) -> T, T>(self, apply: F) -> T {
                    apply(&self.to_be_bytes())
                }

                #[inline]
                fn most_significant_u64(self) -> u64 {
                    $into(self)
                }

                #[inline]
                fn most_significant_u8(self) -> u8 {
                    <$ty>::rotate_left(self, 8) as u8
                }

                #[inline]
                fn unbounded_shr(self, bits: u8) -> Self {
                    <$ty>::unbounded_shr(self, bits as u32)
                }

                #[inline]
                fn leading_zeros(self) -> u8 {
                    <$ty>::leading_zeros(self) as u8
                }

                #[inline]
                fn rotate_left(self, bits: u8) -> Self {
                    <$ty>::rotate_left(self, bits as u32)
                }

                #[inline]
                fn from_most_significant_u64(value: u64) -> Self {
                    $from(value)
                }

                #[inline]
                fn from_u8(value: u8) -> Self {
                    (value as $ty).rotate_right(8)
                }
            }
        )*
    };
}

impl_unsigned_int!(
    u16: 16, |into: u16| {
        (into as u64) << 48
    }, |from: u64| {
        (from >> 48) as u16
    },

    u32: 32, |into: u32| {
        (into as u64) << 32
    }, |from: u64| {
        (from >> 32) as u32
    },

    u64: 64, core::convert::identity, core::convert::identity,

    u128: 128, |into: u128| {
        (into >> 64) as u64
    }, |from: u64| {
        (from as u128) << 64
    },
);

#[cfg(test)]
mod tests {
    use crate::key::tests::take_all;

    #[test]
    fn smoke() {
        take_all_u64(0x1234_5678_9ABC_DEF0u64, &[7, 1]);
    }

    #[test]
    fn take_0() {
        take_all_u64(0x1234_5678_9ABC_DEF0u64, &[0, 1, 0]);
    }

    #[test]
    fn take_1() {
        take_all_u64(0x1234_5678_9ABC_DEF0u64, &[1, 1, 1, 1, 1, 1, 1, 1]);
    }

    fn take_all_u64(key: u64, lens: &[u8]) {
        take_all::<u64>(key.to_be_bytes().as_slice(), key, lens)
    }
}
