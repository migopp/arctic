pub(crate) mod dynamic;
mod fixed;
pub(crate) use fixed::Fixed;

use crate::byte;

pub trait Key {
    #[allow(private_bounds)]
    type Read<'k>: Read + From<Self::Borrow<'k>>
    where
        Self: 'k;

    #[allow(private_bounds)]
    type Write: Write + for<'k> PartialOrd<Self::Borrow<'k>> + for<'k> From<Self::Read<'k>>;

    type Borrow<'k>: Copy
    where
        Self: 'k;

    fn borrow<'k>(&'k self) -> Self::Borrow<'k>;
    fn from_borrowed<'w>(writer: &'w Self::Write) -> Self::Borrow<'w>;
    fn from_owned(writer: Self::Write) -> Self;
}

pub(crate) trait Read: Clone + core::fmt::Debug + Default {
    fn len(&self) -> usize;

    fn peek(&self, len: ribbit::Packed<byte::Len>) -> ribbit::Packed<byte::Array>;

    #[inline]
    fn peek_all(&self) -> ribbit::Packed<byte::Array> {
        self.peek(byte::Array::MAX_LEN.min(self.len()))
    }

    fn take(&mut self, len: ribbit::Packed<byte::Len>) -> ribbit::Packed<byte::Array>;
    fn next(&mut self) -> Option<u8>;
    fn prefix(&self, other: &Self) -> Self;
}

pub(crate) trait Write: Clone + core::fmt::Debug + Default + Eq {
    fn len(&self) -> usize;
    fn extend(&mut self, array: ribbit::Packed<byte::Array>);
    fn push(&mut self, byte: u8);
    fn truncate(&mut self, len: usize);
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
                type Read<'k> = Fixed;
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
        )*
    };
}

impl_unsigned_int!(u8, u16, u32, u64);

impl<const N: usize> Key for [u8; N] {
    type Read<'a> = dynamic::Iter<'a>;
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

impl Key for Vec<u8> {
    type Read<'a> = dynamic::Iter<'a>;
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

impl Key for String {
    type Read<'a> = dynamic::Iter<'a>;
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
