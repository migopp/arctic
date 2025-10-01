pub(crate) mod dynamic;
mod fixed;
pub(crate) use fixed::Fixed;
use ribbit::u3;

use crate::byte;

pub trait Key {
    #[allow(private_bounds)]
    type Iter<'a>: Iterator + PartialOrd<Self::Stack>
    where
        Self: 'a;

    #[allow(private_bounds)]
    type Stack: Stack + for<'a> PartialOrd<Self::Iter<'a>>;

    fn iter<'a>(&'a self) -> Self::Iter<'a>;
}

pub(crate) trait Iterator: Clone + core::fmt::Debug + Default {
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

pub(crate) trait Stack: Clone + core::fmt::Debug + Default {
    fn len(&self) -> usize;
    fn extend(&mut self, array: ribbit::Packed<byte::Array>);
    fn push(&mut self, byte: u8);
    fn truncate(&mut self, len: usize);
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Ignore;

impl Stack for Ignore {
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
                type Iter<'a> = Fixed;
                type Stack = Fixed;
                #[inline]
                fn iter<'a>(&'a self) -> Self::Iter<'a> {
                    Fixed::from(*self)
                }
            }
        )*
    };
}

impl_unsigned_int!(u8, u16, u32, u64);

impl<const N: usize> Key for [u8; N] {
    type Iter<'a> = dynamic::Iter<'a>;
    type Stack = Vec<u8>;
    #[inline]
    fn iter<'a>(&'a self) -> Self::Iter<'a> {
        dynamic::Iter::from(self.as_slice())
    }
}

impl Key for [u8] {
    type Iter<'a> = dynamic::Iter<'a>;
    type Stack = Vec<u8>;
    #[inline]
    fn iter<'a>(&'a self) -> Self::Iter<'a> {
        dynamic::Iter::from(self)
    }
}

impl Key for Vec<u8> {
    type Iter<'a> = dynamic::Iter<'a>;
    type Stack = Vec<u8>;
    #[inline]
    fn iter<'a>(&'a self) -> Self::Iter<'a> {
        dynamic::Iter::from(self.as_slice())
    }
}

impl Key for str {
    type Iter<'a> = dynamic::Iter<'a>;
    type Stack = Vec<u8>;
    #[inline]
    fn iter<'a>(&'a self) -> Self::Iter<'a> {
        dynamic::Iter::from(self.as_bytes())
    }
}

impl Key for String {
    type Iter<'a> = dynamic::Iter<'a>;
    type Stack = Vec<u8>;
    #[inline]
    fn iter<'a>(&'a self) -> Self::Iter<'a> {
        dynamic::Iter::from(self.as_bytes())
    }
}
