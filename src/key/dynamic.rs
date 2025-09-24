use ribbit::u3;

use crate::key;

#[derive(Copy, Clone, Debug)]
pub enum Dynamic<'a> {
    // INVARIANT: `len > 8`
    Large(&'a [u8]),
    Small(key::Fixed),
}

impl<'a> From<&'a [u8]> for Dynamic<'a> {
    #[inline]
    fn from(key: &'a [u8]) -> Self {
        match key.len() {
            9.. => Self::Large(key),
            len => {
                let mut buffer = [0u8; 8];
                buffer[..len].copy_from_slice(key);
                Self::Small(key::Fixed::new(u64::from_ne_bytes(buffer), len as u8))
            }
        }
    }
}

impl Default for Dynamic<'_> {
    #[inline]
    fn default() -> Self {
        Self::Small(key::Fixed::default())
    }
}

impl key::Iterator for Dynamic<'_> {
    #[inline]
    fn len(&self) -> usize {
        match self {
            Dynamic::Large(large) => large.len(),
            Dynamic::Small(small) => small.len(),
        }
    }

    #[inline]
    fn peek(&self, len: u3) -> ribbit::Packed<key::Array> {
        match self {
            Dynamic::Large(large) => {
                validate!(large.len() > 8);
                let buffer = unsafe { large.as_ptr().cast::<u64>().read_unaligned() };
                key::Array::from_u64_truncate(buffer, len)
            }
            Dynamic::Small(small) => small.peek(len),
        }
    }

    #[inline]
    fn take(&mut self, len: u3) -> ribbit::Packed<key::Array> {
        match self {
            Dynamic::Large(large) => {
                validate!(large.len() > 8);
                let buffer = unsafe { large.as_ptr().cast::<u64>().read_unaligned() };
                let array = key::Array::from_u64_truncate(buffer, len);
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

                *self = Self::Small(key::Fixed::new(buffer, after as u8));
                array
            }
            Dynamic::Small(small) => small.take(len),
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
                    Self::Small(key::Fixed::new(
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

#[cfg(test)]
mod tests {
    use ribbit::u3;

    use crate::key::Array;
    use crate::key::Dynamic;
    use crate::key::Iterator as _;

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
        let mut iter = Dynamic::from(initial);

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
