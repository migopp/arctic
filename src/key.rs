pub(crate) mod dynamic;
mod fixed;

use crate::byte;

pub trait Key {
    #[allow(private_bounds)]
    type Read<'k>: Read + From<Self::Borrow<'k>>
    where
        Self: 'k;

    #[allow(private_bounds)]
    type Write: Write + for<'k> PartialOrd<Self::Borrow<'k>> + for<'k> From<Self::Read<'k>>;

    #[allow(private_bounds)]
    type Borrow<'k>: Borrow
    where
        Self: 'k;

    fn borrow<'k>(&'k self) -> Self::Borrow<'k>;
    fn from_borrowed<'w>(writer: &'w Self::Write) -> Self::Borrow<'w>;
    fn from_owned(writer: Self::Write) -> Self;
}

pub(crate) trait Read: Clone + core::fmt::Debug + Default {
    fn remaining_bits(&self) -> usize;

    #[inline]
    fn remaining_bytes(&self) -> usize {
        self.remaining_bits() >> 3
    }

    fn peek(&self, len: ribbit::Packed<byte::Len>) -> ribbit::Packed<byte::Array>;

    #[inline]
    fn peek_all(&self) -> ribbit::Packed<byte::Array> {
        self.peek(byte::Len::MAX.min_bits(self.remaining_bits()))
    }

    fn take(&mut self, len: ribbit::Packed<byte::Len>) -> ribbit::Packed<byte::Array>;
    fn next(&mut self) -> Option<u8>;
    fn prefix(&self, other: &Self) -> Self;
}

pub(crate) trait Write: Clone + core::fmt::Debug + Default + Eq {
    fn bits(&self) -> usize;
    fn extend(&mut self, array: ribbit::Packed<byte::Array>);
    fn push(&mut self, byte: u8);
    fn truncate(&mut self, bits: usize);
}

pub(crate) trait Borrow: Copy + core::fmt::Debug {
    fn slice(self, bits: usize) -> Self;
    fn get(self, bit: usize) -> u8;
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Ignore;

impl Write for Ignore {
    #[inline]
    fn bits(&self) -> usize {
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
                type Read<'k> = fixed::Reader;
                type Write = fixed::Writer;
                type Borrow<'k> = Self;

                #[inline]
                fn borrow(&self) -> Self {
                    *self
                }

                #[inline]
                fn from_borrowed(write: &Self::Write) -> Self {
                    Self::from(*write)
                }

                #[inline]
                fn from_owned(write: Self::Write) -> Self {
                    Self::from(write)
                }
            }

            impl Borrow for $ty {
                #[inline]
                fn get(self, bit: usize) -> u8 {
                    self.rotate_left(8 + bit as u32) as u8
                }

                #[inline]
                fn slice(self, bits: usize) -> Self {
                    self & !(Self::MAX >> bits)
                }
            }
        )*
    };
}

impl_unsigned_int!(u8, u16, u32, u64);

impl<const N: usize> Key for [u8; N] {
    type Read<'a> = dynamic::Reader<'a>;
    type Write = dynamic::Writer;
    type Borrow<'a> = &'a [u8; N];

    #[inline]
    fn borrow<'k>(&'k self) -> Self::Borrow<'k> {
        self
    }

    #[inline]
    fn from_borrowed<'w>(writer: &'w Self::Write) -> Self::Borrow<'w> {
        writer
            .0
            .as_slice()
            .try_into()
            .expect("key::Write should have same length as key::Key")
    }

    #[inline]
    fn from_owned(writer: Self::Write) -> Self {
        writer
            .0
            .try_into()
            .expect("key::Write should have same length as key::Key")
    }
}

impl<const N: usize> Borrow for &'_ [u8; N] {
    #[inline]
    fn get(self, bit: usize) -> u8 {
        self.as_slice().get(bit >> 3).copied().unwrap()
    }

    #[inline]
    fn slice(self, _bits: usize) -> Self {
        todo!()
    }
}

impl Key for Vec<u8> {
    type Read<'a> = dynamic::Reader<'a>;
    type Write = dynamic::Writer;
    type Borrow<'a> = &'a [u8];

    #[inline]
    fn borrow<'k>(&'k self) -> Self::Borrow<'k> {
        self
    }

    #[inline]
    fn from_borrowed<'w>(writer: &'w Self::Write) -> Self::Borrow<'w> {
        writer.0.as_slice()
    }

    #[inline]
    fn from_owned(writer: Self::Write) -> Self {
        writer.0
    }
}

impl Borrow for &'_ [u8] {
    #[inline]
    fn get(self, bit: usize) -> u8 {
        self.get(bit >> 3).copied().unwrap()
    }

    #[inline]
    fn slice(self, bits: usize) -> Self {
        &self[..bits >> 3]
    }
}

impl Key for String {
    type Read<'a> = dynamic::Reader<'a>;
    type Write = dynamic::Writer;
    type Borrow<'a> = &'a str;

    #[inline]
    fn borrow<'k>(&'k self) -> Self::Borrow<'k> {
        self
    }

    #[inline]
    fn from_borrowed<'w>(writer: &'w Self::Write) -> Self::Borrow<'w> {
        str::from_utf8(writer.0.as_slice()).expect("key::Write should be valid UTF-8")
    }

    #[inline]
    fn from_owned(writer: Self::Write) -> Self {
        String::from_utf8(writer.0).expect("key::Write should be valid UTF-8")
    }
}

impl Borrow for &'_ str {
    #[inline]
    fn get(self, bit: usize) -> u8 {
        self.as_bytes().get(bit >> 3).copied().unwrap()
    }

    fn slice(self, _bits: usize) -> Self {
        todo!()
    }
}
