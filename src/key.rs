use crate::byte;

pub trait Key {
    #[allow(private_bounds)]
    type Iter<'a>: byte::Iterator
    where
        Self: 'a;

    #[allow(private_bounds)]
    type Stack: byte::Stack;

    fn iter<'a>(&'a self) -> Self::Iter<'a>;
}

macro_rules! impl_unsigned_int {
    ($($ty:ty),* $(,)?) => {
        $(
            impl Key for $ty {
                type Iter<'a> = byte::Fixed;
                type Stack = byte::Fixed;
                #[inline]
                fn iter<'a>(&'a self) -> Self::Iter<'a> {
                    byte::Fixed::from(*self)
                }
            }
        )*
    };
}

impl_unsigned_int!(u8, u16, u32, u64);

impl<const N: usize> Key for [u8; N] {
    type Iter<'a> = byte::dynamic::Iter<'a>;
    type Stack = Vec<u8>;
    #[inline]
    fn iter<'a>(&'a self) -> Self::Iter<'a> {
        byte::dynamic::Iter::from(self.as_slice())
    }
}

impl Key for [u8] {
    type Iter<'a> = byte::dynamic::Iter<'a>;
    type Stack = Vec<u8>;
    #[inline]
    fn iter<'a>(&'a self) -> Self::Iter<'a> {
        byte::dynamic::Iter::from(self)
    }
}

impl Key for Vec<u8> {
    type Iter<'a> = byte::dynamic::Iter<'a>;
    type Stack = Vec<u8>;
    #[inline]
    fn iter<'a>(&'a self) -> Self::Iter<'a> {
        byte::dynamic::Iter::from(self.as_slice())
    }
}

impl Key for str {
    type Iter<'a> = byte::dynamic::Iter<'a>;
    type Stack = Vec<u8>;
    #[inline]
    fn iter<'a>(&'a self) -> Self::Iter<'a> {
        byte::dynamic::Iter::from(self.as_bytes())
    }
}

impl Key for String {
    type Iter<'a> = byte::dynamic::Iter<'a>;
    type Stack = Vec<u8>;
    #[inline]
    fn iter<'a>(&'a self) -> Self::Iter<'a> {
        byte::dynamic::Iter::from(self.as_bytes())
    }
}
