use ribbit::u3;
use ribbit::Unpack as _;

use crate::byte;

#[derive(Copy, Clone, Debug)]
pub enum Iter<'a> {
    // INVARIANT: `len > 8`
    Large(&'a [u8]),
    Small(byte::fixed::Iter),
}

impl<'a> From<&'a [u8]> for Iter<'a> {
    #[inline]
    fn from(key: &'a [u8]) -> Self {
        match key.len() {
            9.. => Self::Large(key),
            len => {
                let mut buffer = [0u8; 8];
                buffer[..len].copy_from_slice(key);
                Self::Small(byte::fixed::Iter::new(
                    u64::from_ne_bytes(buffer),
                    len as u8,
                ))
            }
        }
    }
}

impl Default for Iter<'_> {
    #[inline]
    fn default() -> Self {
        Self::Small(byte::fixed::Iter::default())
    }
}

impl byte::Iterator for Iter<'_> {
    #[inline]
    fn len(&self) -> usize {
        match self {
            Iter::Large(large) => large.len(),
            Iter::Small(small) => small.len(),
        }
    }

    #[inline]
    fn peek(&self, len: u3) -> ribbit::Packed<byte::Array> {
        validate!(len.value() as usize <= self.len());

        match self {
            Iter::Large(large) => {
                validate!(large.len() > 8);
                let buffer = unsafe { large.as_ptr().cast::<u64>().read_unaligned() };
                byte::Array::from_u64_truncate(buffer, len)
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
                let buffer = unsafe { large.as_ptr().cast::<u64>().read_unaligned() };
                let array = byte::Array::from_u64_truncate(buffer, len);
                let after = large.len() - len.value() as usize;

                if after > 8 {
                    *self = Self::Large(&large[len.value() as usize..]);
                    return array;
                }

                let buffer = unsafe {
                    (&large[large.len() - 8] as *const u8)
                        .cast::<u64>()
                        .read_unaligned()
                };
                let buffer = buffer >> ((8 - after) << 3);

                *self = Self::Small(byte::fixed::Iter::new(buffer, after as u8));
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
                    Self::Small(byte::fixed::Iter::new(
                        unsafe { large[1..].as_ptr().cast::<u64>().read_unaligned() },
                        8,
                    ))
                };

                Some(byte)
            }
            Self::Small(small) => small.next(),
        }
    }
}

impl byte::Stack for Vec<u8> {
    #[inline]
    fn push_array(&mut self, array: ribbit::Packed<byte::Array>) {
        self.extend(array.unpack().bytes());
    }

    #[inline]
    fn push_byte(&mut self, byte: u8) {
        self.push(byte);
    }

    #[inline]
    fn pop(&mut self, count: usize) {
        validate!(self.len() >= count);
        self.truncate(self.len() - count);
    }
}

#[cfg(test)]
mod tests {
    use ribbit::u3;

    use crate::byte::dynamic;
    use crate::byte::Array;
    use crate::byte::Iterator as _;

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
            Array::with_bytes(iter.take(u3::new(len as u8)), None, |a| {
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
