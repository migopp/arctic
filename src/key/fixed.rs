use core::fmt;

use crate::byte;
use crate::key;
use crate::key::Read as _;

trait Int:
    Sized
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

#[derive(Copy, Clone, Default, Eq)]
pub struct Fixed<I> {
    buffer: I,
    bits: u8,
}

#[expect(private_bounds)]
impl<I: Int> Fixed<I> {
    #[inline]
    pub fn new_masked(buffer: I, bits: u8) -> Self {
        unsafe {
            let bits = bits & !0b111;
            Self::new_unchecked(buffer.most_significant(bits), bits)
        }
    }

    #[inline]
    pub unsafe fn new_unchecked(buffer: I, bits: u8) -> Self {
        validate!(bits <= I::BITS);
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

impl<I: Int> key::Read for Fixed<I> {
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
        validate!(bit <= I::BITS as usize - 8);

        self.buffer.rotate_left(bit as u8).most_significant_u8()
    }

    #[inline]
    fn slice(&self, bits: usize) -> Self {
        validate!(bits <= I::BITS as usize);

        let bits = bits as u8;
        Self {
            buffer: self.buffer.most_significant(bits),
            bits,
        }
    }
}

impl<I: Int> key::Write for Fixed<I> {
    type Len = usize;

    #[inline]
    fn bits(&self) -> usize {
        self.bits as usize
    }

    #[inline]
    fn extend(&mut self, array: byte::Array) {
        validate!(self.bits + array.len().bits() <= I::BITS);

        if array.len().bits() == 0 {
            return;
        }

        self.buffer |= I::from_most_significant_u64(array.value() & !0xFF) >> self.bits;
        self.bits += array.len().bits();
    }

    #[inline]
    unsafe fn extend_nonempty_unchecked(&mut self, array: byte::Array) {
        validate!(self.bits + array.len().bits() <= I::BITS);
        validate!(self.bits >= 8);

        if array.len().bits() == 0 {
            return;
        }

        self.buffer |= I::from_most_significant_u64(array.value()) >> self.bits;
        self.bits += array.len().bits();
    }

    #[inline]
    fn push(&mut self, byte: u8) {
        validate!(self.bits <= I::BITS - 8);

        self.buffer |= I::from_u8(byte).shr(self.bits);
        self.bits += 8;
    }

    #[inline]
    fn truncate(&mut self, bits: usize) {
        validate!(self.bits as usize >= bits);
        validate!(bits <= I::BITS as usize);

        let bits = bits as u8;
        self.buffer = self.buffer.most_significant(bits);
        self.bits = bits;
    }
}

impl<I: Int> core::fmt::Debug for Fixed<I> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.with_bytes(|bytes| f.debug_list().entries(bytes).finish())
    }
}

#[expect(clippy::non_canonical_partial_ord_impl)]
impl<I: Ord> PartialOrd<Fixed<I>> for Fixed<I> {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        validate_eq!(self.bits, other.bits);
        Some(self.buffer.cmp(&other.buffer))
    }
}

impl<I: Ord> Ord for Fixed<I> {
    #[inline]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        validate_eq!(self.bits, other.bits);
        self.buffer.cmp(&other.buffer)
    }
}

impl<I: PartialEq> PartialEq<Fixed<I>> for Fixed<I> {
    #[inline]
    fn eq(&self, reader: &Fixed<I>) -> bool {
        self.buffer == reader.buffer
    }
}

macro_rules! impl_unsigned_int {
    ($($ty:ty: $bits:expr, $into:expr, $from:expr),* $(,)?) => {
        $(
            impl From<$ty> for Fixed<$ty> {
                #[inline]
                fn from(value: $ty) -> Self {
                    Self {
                        buffer: value,
                        bits: $bits,
                    }
                }
            }

            impl From<Fixed<$ty>> for $ty {
                #[inline]
                fn from(fixed: Fixed<$ty>) -> Self {
                    validate_eq!(fixed.bits, <$ty>::BITS as u8);
                    fixed.buffer
                }
            }

            impl Int for $ty {
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
