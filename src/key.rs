pub mod dynamic;
pub mod fixed;

use crate::byte;

pub trait Key: From<Self::Write> + 'static {
    #[allow(private_bounds)]
    type Borrow<'k>: Copy
        + From<&'k Self::Write>
        + for<'a> PartialEq<Self::Borrow<'a>>
        + for<'a> PartialOrd<Self::Borrow<'a>>;

    #[allow(private_bounds)]
    type Read<'k>: Read + From<Self::Borrow<'k>>;

    #[allow(private_bounds)]
    type Write: Write<Len = usize>
        + for<'k> PartialOrd<Self::Read<'k>>
        + for<'k> From<Self::Read<'k>>
        + Ord;

    fn borrow<'k>(&'k self) -> Self::Borrow<'k>;
}

pub(crate) trait Read: Copy + core::fmt::Debug + Default {
    fn bits(&self) -> usize;

    #[inline]
    fn bytes(&self) -> usize {
        self.bits() >> 3
    }

    fn peek(&self, len: byte::Len) -> byte::Array;

    #[inline]
    fn peek_all(&self) -> byte::Array {
        self.peek(byte::Len::MAX.min_bits(self.bits()))
    }

    fn take(&mut self, len: byte::Len) -> byte::Array;

    fn get(&self, bit: usize) -> u8;
    fn slice(&self, bit: usize) -> Self;

    fn next(&mut self) -> Option<u8>;
    fn prefix(&self, other: &Self) -> Self;
}

pub(crate) trait Write: Clone + core::fmt::Debug + Default + Eq {
    type Len: Copy;

    fn bits(&self) -> Self::Len;
    fn extend(&mut self, array: byte::Array);

    /// # SAFETY
    ///
    /// Caller must guarantee `self.bits() > 0`.
    unsafe fn extend_nonempty_unchecked(&mut self, array: byte::Array) {
        self.extend(array)
    }

    fn push(&mut self, byte: u8);
    fn truncate(&mut self, bits: Self::Len);
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ignore;

impl Write for Ignore {
    type Len = ();

    #[inline]
    fn bits(&self) -> Self::Len {}

    #[inline]
    fn extend(&mut self, _array: byte::Array) {}

    #[inline]
    fn push(&mut self, _byte: u8) {}

    #[inline]
    fn truncate(&mut self, (): Self::Len) {}
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
            }

            impl<'k> From<&'k fixed::Writer> for $ty {
                #[inline]
                fn from(writer: &'k fixed::Writer) -> Self {
                    Self::from(*writer)
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
}

impl<'w, const N: usize> From<&'w dynamic::Writer> for &'w [u8; N] {
    #[inline]
    fn from(writer: &'w dynamic::Writer) -> Self {
        writer.0.as_slice().try_into().unwrap()
    }
}

impl<const N: usize> From<dynamic::Writer> for [u8; N] {
    #[inline]
    fn from(writer: dynamic::Writer) -> Self {
        writer
            .0
            .try_into()
            .expect("key::Write should have same length as key::Key")
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
}

impl<'w> From<&'w dynamic::Writer> for &'w [u8] {
    #[inline]
    fn from(writer: &'w dynamic::Writer) -> Self {
        writer.0.as_slice()
    }
}

impl From<dynamic::Writer> for Vec<u8> {
    #[inline]
    fn from(writer: dynamic::Writer) -> Self {
        writer.0
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
}

impl<'w> From<&'w dynamic::Writer> for &'w str {
    #[inline]
    fn from(writer: &'w dynamic::Writer) -> Self {
        str::from_utf8(writer.0.as_slice()).expect("key::Write should be valid UTF-8")
    }
}

impl From<dynamic::Writer> for String {
    #[inline]
    fn from(writer: dynamic::Writer) -> Self {
        String::from_utf8(writer.0).expect("key::Write should be valid UTF-8")
    }
}

#[cfg(test)]
mod tests {
    use crate::byte;
    use crate::key::Read as _;
    use crate::Key;

    pub(super) fn take_all<'k, K: Key>(array: &[u8], key: impl Into<K::Borrow<'k>>, lens: &[u8]) {
        let mut reader = K::Read::from(key.into());

        let mut index = 0;

        for len in lens
            .iter()
            .copied()
            .map(byte::Len::from_bytes)
            .map(Option::unwrap)
        {
            assert_eq!(reader.bytes(), array.len() - index);

            byte::Array::with_bytes(reader.take(len), |actual| {
                assert_eq!(actual, &array[index..][..len.bytes() as usize]);
            });

            index += len.bytes() as usize;
        }

        assert_eq!(reader.bytes(), array.len() - index);
        assert_eq!(reader.next(), array.get(index).copied());
    }
}
