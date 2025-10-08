use core::cmp;

use crate::byte;
use crate::key;
use crate::key::fixed;

#[derive(Copy, Clone, Debug)]
pub enum Reader<'a> {
    // INVARIANT: `len > 8`
    Large(&'a [u8]),
    Small(fixed::Reader),
}

impl<'a> From<&'a [u8]> for Reader<'a> {
    #[inline]
    fn from(key: &'a [u8]) -> Self {
        match key.len() {
            9.. => Self::Large(key),
            len => {
                let mut buffer = [0u8; 8];
                buffer[..len].copy_from_slice(key);
                Self::Small(fixed::Reader::new(
                    u64::from_be_bytes(buffer),
                    (len << 3) as u8,
                ))
            }
        }
    }
}

impl<'a, const N: usize> From<&'a [u8; N]> for Reader<'a> {
    #[inline]
    fn from(value: &'a [u8; N]) -> Self {
        Self::from(value.as_slice())
    }
}

impl<'a> From<&'a str> for Reader<'a> {
    #[inline]
    fn from(value: &'a str) -> Self {
        Self::from(value.as_bytes())
    }
}

impl Default for Reader<'_> {
    #[inline]
    fn default() -> Self {
        Self::Small(fixed::Reader::default())
    }
}

impl key::Read for Reader<'_> {
    #[inline]
    fn bits(&self) -> usize {
        match self {
            Reader::Large(large) => large.len() << 3,
            Reader::Small(small) => key::Read::bits(small),
        }
    }

    #[inline]
    fn peek(&self, len: byte::Len) -> byte::Array {
        validate!(len.bits() as usize <= self.bits());

        match self {
            Reader::Large(large) => unsafe { read_array(large, len) },
            Reader::Small(small) => small.peek(len),
        }
    }

    #[inline]
    fn take(&mut self, len: byte::Len) -> byte::Array {
        validate!(len.bits() as usize <= self.bits());

        match self {
            Reader::Large(large) => {
                validate!(large.len() > 8);

                let array = unsafe { read_array(large, len) };
                let after = (large.len() << 3) - len.bits() as usize;

                if after > 64 {
                    *self = Self::Large(&large[len.bytes() as usize..]);
                    return array;
                }

                let buffer = unsafe {
                    (&large[large.len() - 8] as *const u8)
                        .cast::<u64>()
                        .read_unaligned()
                }
                .to_be();

                *self = Self::Small(fixed::Reader::new(buffer << (64 - after), after as u8));
                array
            }
            Reader::Small(small) => small.take(len),
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
                    Self::Small(fixed::Reader::new(
                        unsafe { large[1..].as_ptr().cast::<u64>().read_unaligned() }.to_be(),
                        64,
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
                (fixed::Reader::new(buffer, 64), small)
            }
        };

        Self::Small(left.prefix(right))
    }

    #[inline]
    fn get(&self, bit: usize) -> u8 {
        match self {
            Reader::Large(large) => large[bit >> 3],
            Reader::Small(small) => small.get(bit),
        }
    }

    #[inline]
    fn slice(&self, bit: usize) -> Self {
        match self {
            Reader::Large(large) => Reader::from(&large[..bit >> 3]),
            Reader::Small(small) => Reader::Small(small.slice(bit)),
        }
    }
}

impl Reader<'_> {
    #[inline]
    fn with_bytes<F: FnOnce(&[u8]) -> T, T>(&self, with: F) -> T {
        match self {
            Reader::Large(large) => with(large),
            Reader::Small(small) => small.with_bytes(with),
        }
    }
}

/// # SAFETY
///
/// Caller must ensure `slice.len() >= 8`
unsafe fn read_array(slice: &[u8], len: byte::Len) -> byte::Array {
    validate!(slice.len() >= 8);

    let buffer = unsafe { slice.as_ptr().cast::<u64>().read_unaligned() };
    byte::Array::from_u64_truncate(buffer.to_be(), len)
}

#[repr(transparent)]
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct Writer(pub(super) Vec<u8>);

impl key::Write for Writer {
    type Len = usize;

    #[inline]
    fn bits(&self) -> usize {
        self.0.len() << 3
    }

    #[inline]
    fn extend(&mut self, array: byte::Array) {
        self.0.extend(array)
    }

    #[inline]
    fn push(&mut self, byte: u8) {
        self.0.push(byte)
    }

    #[inline]
    fn truncate(&mut self, bits: usize) {
        self.0.truncate(bits >> 3);
    }
}

impl<'a> From<Reader<'a>> for Writer {
    fn from(iter: Reader<'a>) -> Self {
        Self(match iter {
            Reader::Large(large) => large.to_vec(),
            Reader::Small(small) => small.with_bytes(|small| small.to_vec()),
        })
    }
}

impl PartialEq<Reader<'_>> for Writer {
    #[inline]
    fn eq(&self, other: &Reader<'_>) -> bool {
        other.with_bytes(|other| self.0 == other)
    }
}

impl PartialOrd<Reader<'_>> for Writer {
    #[inline]
    fn partial_cmp(&self, other: &Reader<'_>) -> Option<cmp::Ordering> {
        other.with_bytes(|other| self.0.as_slice().partial_cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use crate::key::tests::take_all;

    #[test]
    fn smoke() {
        take_all_array(b"0123456789", &[1])
    }

    #[test]
    fn take_0() {
        take_all_array(b"", &[0])
    }

    #[test]
    fn take_1() {
        take_all_array(b"0", &[1])
    }

    #[test]
    fn len_3() {
        take_all_array(b"012", &[1, 1, 1])
    }

    #[test]
    fn len_5() {
        take_all_array(b"01234", &[1, 1, 1, 1, 1])
    }

    #[test]
    fn len_7() {
        take_all_array(b"0123456", &[1, 1, 1, 1, 1, 1, 1])
    }

    #[test]
    fn switch_exact() {
        take_all_array(b"0123456789", &[2, 2])
    }

    #[test]
    fn switch_inexact() {
        take_all_array(b"0123456789", &[4, 2])
    }

    #[test]
    fn long() {
        take_all_array(b"abcdefghijklmnopqrstuvwxyz", &[1, 2, 3, 4, 5, 4, 3, 2, 1])
    }

    fn take_all_array(key: &[u8], lens: &[u8]) {
        take_all::<Vec<u8>>(key, key, lens)
    }
}
