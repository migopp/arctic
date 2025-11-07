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
    + core::ops::Shr<usize, Output = Self>
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

    fn most_significant_u128(self) -> u128;
    fn most_significant_u64(self) -> u64;
    fn most_significant_u8(self) -> u8;

    #[inline]
    fn most_significant(self, bits: u8) -> Self {
        Self::MAX.unbounded_shr(bits).not().bitand(self)
    }

    fn shl_at_most_56(self, bits: u8) -> Self;
    fn unbounded_shr(self, bits: u8) -> Self;
    fn leading_zeros(self) -> u8;

    fn from_most_significant_u64(value: u64) -> Self;
    fn from_u8(value: u8) -> Self;
}

#[derive(Copy, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Reader<U> {
    buffer: U,
    bits: u8,
}

#[expect(private_bounds)]
impl<U: Uint> Reader<U> {
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

impl<U: Uint> key::Read for Reader<U> {
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
    fn hazard(&self) -> ribbit::Packed<crate::concurrent::hazard::prefix::Be> {
        crate::concurrent::hazard::prefix::Be::new_hazard(
            self.buffer.most_significant_u128(),
            if U::BYTES < 16 {
                self.bits as usize
            } else {
                self.bits.min(120) as usize
            },
        )
    }

    #[inline]
    fn take(&mut self, len: byte::Len) -> byte::Array {
        validate!(len.bits() as usize <= self.bits());
        validate_eq!(len.bits() & 0b111, 0);

        if len.bits() == 0 {
            return byte::Array::EMPTY;
        }

        let array = self.peek(len);
        self.buffer = self.buffer.shl_at_most_56(len.bits());
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

    fn seek(&mut self, bits: usize) {
        validate!(self.bits() >= bits);

        if self.bits as usize == bits {
            self.buffer = U::default();
            self.bits = 0;
        } else {
            let bits = bits as u8;
            self.buffer = self.buffer.shl_at_most_56(bits);
            self.bits -= bits;
        }
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
    fn slice(&self, bits: usize) -> Self {
        validate!(bits <= U::BITS as usize);

        let bits = bits as u8;
        Self {
            buffer: self.buffer.most_significant(bits),
            bits,
        }
    }
}

impl<U: Uint> core::fmt::Debug for Reader<U> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.with_bytes(|bytes| f.debug_list().entries(bytes).finish())
    }
}

#[repr(transparent)]
#[derive(Copy, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Writer<U>(U);

impl<U> Writer<U> {
    pub(super) fn into_key_unchecked(self) -> U {
        self.0
    }
}

impl<U: Uint> key::Write for Writer<U> {
    type Len = usize;

    #[inline]
    fn len_from_bits(bits: usize) -> Self::Len {
        bits
    }

    #[inline]
    fn extend(&mut self, bits: &mut usize, array: byte::Array) {
        validate!(*bits + array.len().bits() as usize <= U::BITS as usize);

        if array.len().bits() == 0 {
            return;
        }

        self.0 |= U::from_most_significant_u64(array.value() & !0xFF) >> *bits;
        *bits += array.len().bits() as usize;
    }

    #[inline]
    unsafe fn extend_nonempty_unchecked(&mut self, bits: &mut usize, array: byte::Array) {
        validate!(*bits + array.len().bits() as usize <= U::BITS as usize);
        validate!(*bits >= 8);

        if array.len().bits() == 0 {
            return;
        }

        self.0 |= U::from_most_significant_u64(array.value()) >> *bits;
        *bits += array.len().bits() as usize;
    }

    #[inline]
    fn push(&mut self, bits: &mut usize, byte: u8) {
        validate!(*bits <= U::BITS as usize - 8);

        self.0 |= U::from_u8(byte).shr(*bits);
        *bits += 8;
    }

    #[inline]
    fn truncate(&mut self, bits: usize) {
        validate!(bits <= U::BITS as usize);

        self.0 = self.0.most_significant(bits as u8);
    }
}

impl<U: Uint> core::fmt::Debug for Writer<U> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0
            .with_be_bytes(|bytes| f.debug_list().entries(bytes).finish())
    }
}

impl<U> From<Reader<U>> for Writer<U> {
    fn from(reader: Reader<U>) -> Self {
        Self(reader.buffer)
    }
}

macro_rules! impl_unsigned_int {
    ($($ty:ty: $bits:expr, $into_u64:expr, $from_u64:expr, $into_u128:expr),* $(,)?) => {
        $(
            impl From<$ty> for Reader<$ty> {
                #[inline]
                fn from(value: $ty) -> Self {
                    Self {
                        buffer: value,
                        bits: $bits,
                    }
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
                fn most_significant_u128(self) -> u128 {
                    $into_u128(self)
                }

                #[inline]
                fn most_significant_u64(self) -> u64 {
                    $into_u64(self)
                }

                #[inline]
                fn most_significant_u8(self) -> u8 {
                    <$ty>::rotate_left(self, 8) as u8
                }

                #[inline]
                fn shl_at_most_56(self, bits: u8) -> Self {
                    if <$ty>::BITS <= 56 {
                        <$ty>::unbounded_shl(self, bits as u32)
                    } else {
                        self << bits
                    }
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
                fn from_most_significant_u64(value: u64) -> Self {
                    $from_u64(value)
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
    u16: 16, |from: Self| {
        (from as u64) << 48
    }, |into: u64| {
        (into >> 48) as Self
    }, |from: Self| {
        (from as u128) << 112
    },

    u32: 32, |from: Self| {
        (from as u64) << 32
    }, |into: u64| {
        (into >> 32) as Self
    }, |from: Self| {
        (from as u128) << 96
    },

    u64: 64, core::convert::identity, core::convert::identity, |from: Self| {
        (from as u128) << 64
    },

    u128: 128, |into: u128| {
        (into >> 64) as u64
    }, |from: u64| {
        (from as u128) << 64
    }, core::convert::identity,
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
