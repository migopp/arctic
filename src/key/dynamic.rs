use core::cmp;

use ribbit::u3;

use crate::byte;
use crate::key;
use crate::key::Fixed;

#[derive(Copy, Clone, Debug)]
pub enum Iter<'a> {
    // INVARIANT: `len > 8`
    Large(&'a [u8]),
    Small(Fixed),
}

impl<'a> From<&'a [u8]> for Iter<'a> {
    #[inline]
    fn from(key: &'a [u8]) -> Self {
        match key.len() {
            9.. => Self::Large(key),
            len => {
                let mut buffer = [0u8; 8];
                buffer[..len].copy_from_slice(key);
                Self::Small(Fixed::new(u64::from_be_bytes(buffer), len as u8))
            }
        }
    }
}

impl<'a, const N: usize> From<&'a [u8; N]> for Iter<'a> {
    #[inline]
    fn from(value: &'a [u8; N]) -> Self {
        Self::from(value.as_slice())
    }
}

impl<'a> From<&'a str> for Iter<'a> {
    #[inline]
    fn from(value: &'a str) -> Self {
        Self::from(value.as_bytes())
    }
}

impl Default for Iter<'_> {
    #[inline]
    fn default() -> Self {
        Self::Small(Fixed::default())
    }
}

impl key::Read for Iter<'_> {
    #[inline]
    fn len(&self) -> usize {
        match self {
            Iter::Large(large) => large.len(),
            Iter::Small(small) => key::Read::len(small),
        }
    }

    #[inline]
    fn peek(&self, len: u3) -> ribbit::Packed<byte::Array> {
        validate!(len.value() as usize <= self.len());

        match self {
            Iter::Large(large) => {
                validate!(large.len() > 8);
                let buffer = unsafe { large.as_ptr().cast::<u64>().read_unaligned() }.to_be();
                ribbit::Packed::<byte::Array>::from_u64_truncate(buffer, len)
            }
            Iter::Small(small) => small.peek(len),
        }
    }

    #[inline]
    fn take(&mut self, len: u3) -> ribbit::Packed<byte::Array> {
        validate!(len.value() as usize <= self.len());

        match self {
            Iter::Large(large) => {
                validate!(large.len() > 8);
                let buffer = unsafe { large.as_ptr().cast::<u64>().read_unaligned() }.to_be();
                let array = ribbit::Packed::<byte::Array>::from_u64_truncate(buffer, len);
                let after = large.len() - len.value() as usize;

                if after > 8 {
                    *self = Self::Large(&large[len.value() as usize..]);
                    return array;
                }

                let buffer = unsafe {
                    (&large[large.len() - 8] as *const u8)
                        .cast::<u64>()
                        .read_unaligned()
                }
                .to_be();
                let shift = (8 - after) << 3;
                *self = Self::Small(Fixed::new(buffer << shift, after as u8));
                array
            }
            Iter::Small(small) => small.take(len),
        }
    }

    #[inline]
    fn next(&mut self) -> Option<u8> {
        match self {
            Self::Large(large) => {
                validate!(large.len() > 8);

                let byte = large[0];

                *self = if large.len() - 1 > 8 {
                    Self::Large(&large[1..])
                } else {
                    Self::Small(Fixed::new(
                        unsafe { large[1..].as_ptr().cast::<u64>().read_unaligned() }.to_be(),
                        8,
                    ))
                };

                Some(byte)
            }
            Self::Small(small) => small.next(),
        }
    }

    fn prefix(&self, other: &Self) -> Self {
        let (left, right) = match (self, other) {
            (Self::Large(left), Self::Large(right)) => {
                let index = core::iter::zip(*left, *right)
                    .position(|(l, r)| l != r)
                    .unwrap_or_else(|| left.len().min(right.len()));
                return Self::from(&left[index..]);
            }
            (Self::Small(left), Self::Small(right)) => (*left, right),
            (Self::Small(small), Self::Large(large)) | (Self::Large(large), Self::Small(small)) => {
                // SAFETY: `large.len() > 8`
                let buffer = unsafe { large.as_ptr().cast::<u64>().read_unaligned() }.to_be();
                (Fixed::new(buffer, 8), small)
            }
        };

        Self::Small(left.prefix(right))
    }
}

#[repr(transparent)]
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct Writer(pub(super) Vec<u8>);

impl key::Write for Writer {
    #[inline]
    fn len(&self) -> usize {
        self.0.len()
    }

    #[inline]
    fn extend(&mut self, array: ribbit::Packed<byte::Array>) {
        self.0.extend(array)
    }

    #[inline]
    fn push(&mut self, byte: u8) {
        self.0.push(byte)
    }

    #[inline]
    fn truncate(&mut self, len: usize) {
        self.0.truncate(len);
    }
}

impl<'a> From<Iter<'a>> for Writer {
    fn from(iter: Iter<'a>) -> Self {
        Self(match iter {
            Iter::Large(large) => large.to_vec(),
            Iter::Small(small) => small.with_bytes(|small| small.to_vec()),
        })
    }
}

impl<T: AsRef<[u8]>> PartialEq<T> for Writer {
    #[inline]
    fn eq(&self, other: &T) -> bool {
        self.0 == other.as_ref()
    }
}

impl<T: AsRef<[u8]>> PartialOrd<T> for Writer {
    #[inline]
    fn partial_cmp(&self, other: &T) -> Option<cmp::Ordering> {
        self.0.as_slice().partial_cmp(other.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use ribbit::u3;

    use crate::byte::Array;
    use crate::key::dynamic;
    use crate::key::Read as _;

    #[test]
    fn dynamic_smoke() {
        take_all(b"0123456789", [1])
    }

    #[test]
    fn dynamic_0() {
        take_all(b"", [0])
    }

    #[test]
    fn dynamic_1() {
        take_all(b"0", [1])
    }

    #[test]
    fn dynamic_3() {
        take_all(b"012", [1, 1, 1])
    }

    #[test]
    fn dynamic_5() {
        take_all(b"01234", [1, 1, 1, 1, 1])
    }

    #[test]
    fn dynamic_7() {
        take_all(b"0123456", [1, 1, 1, 1, 1, 1, 1])
    }

    #[test]
    fn dynamic_switch_exact() {
        take_all(b"0123456789", [2, 2])
    }

    #[test]
    fn dynamic_switch_inexact() {
        take_all(b"0123456789", [4, 2])
    }

    #[test]
    fn dynamic_long() {
        take_all(b"abcdefghijklmnopqrstuvwxyz", [1, 2, 3, 4, 5, 4, 3, 2, 1])
    }

    fn take_all<I: IntoIterator<Item = usize>>(initial: &[u8], lens: I) {
        let mut iter = dynamic::Iter::from(initial);

        let mut index = 0;
        for len in lens {
            assert_eq!(iter.len(), initial.len() - index);
            ribbit::Packed::<Array>::with_bytes(iter.take(u3::new(len as u8)), |a| {
                assert_eq!(a, &initial[index..][..len]);
            });
            index += len;
        }

        assert_eq!(iter.len(), initial.len() - index);
        if iter.len() > 0 {
            assert_eq!(iter.next(), Some(initial[index]));
        } else {
            assert_eq!(iter.next(), None);
        }
    }
}
