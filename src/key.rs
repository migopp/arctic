pub(crate) mod dynamic;
mod fixed;
pub(crate) use fixed::Fixed;
use ribbit::u3;

use crate::byte;

pub trait Key {
    #[allow(private_bounds)]
    type Read<'a>: Read
    where
        Self: 'a;

    #[allow(private_bounds)]
    type Write: Write + for<'a> PartialOrd<Self::Read<'a>> + for<'a> From<Self::Read<'a>>;

    fn read<'a>(&'a self) -> Self::Read<'a>;
}

pub(crate) trait Read: Clone + core::fmt::Debug + Default {
    fn len(&self) -> usize;

    fn peek(&self, len: u3) -> ribbit::Packed<byte::Array>;

    #[inline]
    fn peek_all(&self) -> ribbit::Packed<byte::Array> {
        self.peek(byte::Array::min_len(self.len(), byte::Array::MAX_LEN))
    }

    fn take(&mut self, len: u3) -> ribbit::Packed<byte::Array>;
    fn next(&mut self) -> Option<u8>;

    #[allow(dead_code)]
    fn prefix(&self, other: &Self) -> Self;
}

pub(crate) trait Write: Clone + core::fmt::Debug + Default {
    fn len(&self) -> usize;
    fn extend(&mut self, array: ribbit::Packed<byte::Array>);
    fn push(&mut self, byte: u8);
    fn truncate(&mut self, len: usize);
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Ignore;

impl Write for Ignore {
    #[inline]
    fn len(&self) -> usize {
        0
    }

    #[inline]
    fn extend(&mut self, _array: ribbit::Packed<byte::Array>) {}

    #[inline]
    fn push(&mut self, _byte: u8) {}

    #[inline]
    fn truncate(&mut self, _len: usize) {}
}

macro_rules! impl_unsigned_int {
    ($($ty:ty),* $(,)?) => {
        $(
            impl Key for $ty {
                type Read<'a> = Fixed;
                type Write = Fixed;
                #[inline]
                fn read<'a>(&'a self) -> Self::Read<'a> {
                    Fixed::from(*self)
                }
            }
        )*
    };
}

impl_unsigned_int!(u8, u16, u32, u64);

impl<const N: usize> Key for [u8; N] {
    type Read<'a> = dynamic::Iter<'a>;
    type Write = Vec<u8>;
    #[inline]
    fn read<'a>(&'a self) -> Self::Read<'a> {
        dynamic::Iter::from(self.as_slice())
    }
}

impl Key for [u8] {
    type Read<'a> = dynamic::Iter<'a>;
    type Write = Vec<u8>;
    #[inline]
    fn read<'a>(&'a self) -> Self::Read<'a> {
        dynamic::Iter::from(self)
    }
}

impl Key for Vec<u8> {
    type Read<'a> = dynamic::Iter<'a>;
    type Write = Vec<u8>;
    #[inline]
    fn read<'a>(&'a self) -> Self::Read<'a> {
        dynamic::Iter::from(self.as_slice())
    }
}

impl Key for str {
    type Read<'a> = dynamic::Iter<'a>;
    type Write = Vec<u8>;
    #[inline]
    fn read<'a>(&'a self) -> Self::Read<'a> {
        dynamic::Iter::from(self.as_bytes())
    }
}

impl Key for String {
    type Read<'a> = dynamic::Iter<'a>;
    type Write = Vec<u8>;
    #[inline]
    fn read<'a>(&'a self) -> Self::Read<'a> {
        dynamic::Iter::from(self.as_bytes())
    }
}
