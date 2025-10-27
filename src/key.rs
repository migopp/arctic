pub mod dynamic;
pub mod fixed;

use core::fmt;
use core::marker::PhantomData;

use crate::byte;

pub trait Key: 'static {
    #[allow(private_bounds)]
    type Borrow<'k>: Copy;

    #[allow(private_bounds)]
    type Read<'k>: Read + From<Self::Borrow<'k>> + From<&'k Self::Write>;

    #[allow(private_bounds)]
    type Write: Write<Len = usize> + for<'k> From<Self::Read<'k>>;

    /// HACK: work around invariant lifetime for generic associated traits
    /// https://users.rust-lang.org/t/expressing-the-covariance-of-gats/65664/4
    fn reborrow<'long, 'short>(reader: Self::Read<'long>) -> Self::Read<'short>
    where
        'long: 'short;

    unsafe fn borrow_writer_unchecked<'w>(writer: &'w Self::Write) -> Self::Borrow<'w>;

    unsafe fn from_writer_unchecked(writer: Self::Write) -> Self;

    fn borrow<'k>(&'k self) -> Self::Borrow<'k>;
}

pub(crate) trait Read: Copy + fmt::Debug + Default + Ord {
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

pub(crate) trait Write: Clone + fmt::Debug + Default + Ord {
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

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
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

#[derive(Copy, Clone, PartialOrd, PartialEq)]
pub struct Fixed<K, U> {
    key: K,
    _uint: PhantomData<U>,
}

impl<K: Copy, U> Fixed<K, U> {
    #[inline]
    pub const fn new(key: K) -> Self {
        Self {
            key,
            _uint: PhantomData,
        }
    }

    #[inline]
    pub const fn key(self) -> K {
        self.key
    }
}

impl<K, U> Key for Fixed<K, U>
where
    K: 'static + Copy + From<U> + PartialOrd,
    U: From<K> + fixed::Uint + From<fixed::Buffer<U>>,
    fixed::Buffer<U>: From<U>,
{
    type Borrow<'k> = Self;
    type Read<'k> = fixed::Buffer<U>;
    type Write = fixed::Buffer<U>;

    #[inline]
    fn reborrow<'long, 'short>(reader: Self::Read<'long>) -> Self::Read<'short>
    where
        'long: 'short,
    {
        reader
    }

    #[inline]
    fn borrow<'k>(&'k self) -> Self::Borrow<'k> {
        *self
    }

    #[inline]
    unsafe fn borrow_writer_unchecked<'w>(writer: &'w Self::Write) -> Self::Borrow<'w> {
        Self::from_writer_unchecked(*writer)
    }

    #[inline]
    unsafe fn from_writer_unchecked(writer: Self::Write) -> Self {
        Self {
            key: K::from(U::from(writer)),
            _uint: PhantomData,
        }
    }
}

impl<K, U> From<Fixed<K, U>> for fixed::Buffer<U>
where
    fixed::Buffer<U>: From<U>,
    U: fixed::Uint + From<K>,
{
    #[inline]
    fn from(fixed: Fixed<K, U>) -> Self {
        Self::from(U::from(fixed.key))
    }
}

macro_rules! impl_unsigned_int {
    ($($ty:ty),* $(,)?) => {
        $(
            impl Key for $ty {
                type Read<'k> = fixed::Buffer<$ty>;
                type Write = fixed::Buffer<$ty>;
                type Borrow<'k> = Self;

                #[inline]
                fn borrow(&self) -> Self {
                    *self
                }

                #[inline]
                fn reborrow<'long, 'short>(reader: Self::Read<'long>) -> Self::Read<'short>
                where
                    'long: 'short
                {
                    reader
                }

                #[inline]
                unsafe fn borrow_writer_unchecked<'w>(writer: &'w Self::Write) -> Self::Borrow<'w> {
                    writer.into_value_unchecked()
                }

                #[inline]
                unsafe fn from_writer_unchecked(writer: Self::Write) -> Self {
                    writer.into_value_unchecked()
                }
            }
        )*
    };
}

impl_unsigned_int!(u16, u32, u64, u128);

impl Key for Vec<u8> {
    type Read<'k> = dynamic::Reader<'k>;
    type Write = dynamic::Writer;
    type Borrow<'k> = &'k [u8];

    #[inline]
    fn reborrow<'long, 'short>(reader: Self::Read<'long>) -> Self::Read<'short>
    where
        'long: 'short,
    {
        reader
    }

    #[inline]
    fn borrow<'k>(&'k self) -> Self::Borrow<'k> {
        self
    }

    #[inline]
    unsafe fn borrow_writer_unchecked<'w>(writer: &'w Self::Write) -> Self::Borrow<'w> {
        &writer.0
    }

    #[inline]
    unsafe fn from_writer_unchecked(writer: Self::Write) -> Self {
        writer.0
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
    type Read<'k> = dynamic::Reader<'k>;
    type Write = dynamic::Writer;
    type Borrow<'k> = &'k str;

    #[inline]
    fn reborrow<'long, 'short>(reader: Self::Read<'long>) -> Self::Read<'short>
    where
        'long: 'short,
    {
        reader
    }

    #[inline]
    fn borrow<'k>(&'k self) -> Self::Borrow<'k> {
        self
    }

    #[inline]
    unsafe fn borrow_writer_unchecked<'w>(writer: &'w Self::Write) -> Self::Borrow<'w> {
        if cfg!(feature = "validate") {
            core::str::from_utf8(&writer.0).unwrap()
        } else {
            unsafe { core::str::from_utf8_unchecked(&writer.0) }
        }
    }

    #[inline]
    unsafe fn from_writer_unchecked(writer: Self::Write) -> Self {
        if cfg!(feature = "validate") {
            String::from_utf8(writer.0).unwrap()
        } else {
            unsafe { String::from_utf8_unchecked(writer.0) }
        }
    }
}

impl<'w> From<&'w dynamic::Writer> for &'w str {
    #[inline]
    fn from(writer: &'w dynamic::Writer) -> Self {
        str::from_utf8(writer.0.as_slice()).expect("key::Write should be valid UTF-8")
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
