use core::fmt::Debug;
use core::ops::BitAnd as _;
use core::ops::BitOr as _;
use core::ops::BitXor as _;
use core::ops::Shr as _;

use ribbit::u3;
use ribbit::u56;

#[derive(Copy, Clone, Default, PartialEq, Eq)]
#[ribbit::pack(size = 59, debug, eq)]
pub(crate) struct Array {
    #[ribbit(size = 56)]
    buffer: u56,

    #[ribbit(size = 3)]
    pub(crate) len: u3,
}

impl Array {
    pub(crate) const EMPTY: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(u56::new(0), u3::new(0));

    const MASK: u64 = 0x00FF_FFFF_FFFF_FFFF;
    pub(crate) const MAX: usize = 7;

    #[inline]
    pub(crate) fn from_slice<K: Iterator>(mut key: K) -> ribbit::Packed<Self> {
        let len = unsafe { u3::new_unchecked(key.len().min(Self::MAX) as u8) };
        key.take(len)
    }

    #[inline]
    pub(crate) fn match_prefix<K: Iterator>(key: &mut K, edge: ribbit::Packed<Self>) -> bool {
        let len = unsafe { u3::new_unchecked(key.len().min(edge.len().value() as usize) as u8) };
        key.take(len) == edge
    }

    #[inline]
    pub(crate) fn match_split<K: Iterator>(key: &mut K, edge: ribbit::Packed<Self>) -> Match {
        let edge_len = edge.len().value() as usize;
        let key_len = key.len();
        let len = unsafe { u3::new_unchecked(key_len.min(edge_len) as u8) };
        let key = key.take(len);
        if key == edge {
            return Match::Full;
        }

        let edge = edge.value.value();
        let prefix_byte = key
            .value
            .value()
            .bitxor(edge)
            // Guarantee `trailing_zeros` cannot produce more than `len * 8` bits
            .bitor(1u64 << ((len.value() as u64) << 3))
            .trailing_zeros()
            .shr(3u32) as u8;

        let prefix_bit = prefix_byte << 3;

        Match::Partial {
            start: unsafe {
                ribbit::Packed::<Self>::new(
                    u56::new_unchecked(edge & ((1u64 << prefix_bit) - 1u64)),
                    u3::new_unchecked(prefix_byte),
                )
            },
            middle: (edge >> prefix_bit) as u8,
            end: unsafe {
                ribbit::Packed::<Self>::new(
                    u56::new_unchecked((edge & Self::MASK) >> (prefix_bit + 8)),
                    u3::new_unchecked(edge_len as u8 - prefix_byte - 1),
                )
            },
        }
    }

    pub(crate) fn compress(
        parent: ribbit::Packed<Self>,
        byte: u8,
        child: ribbit::Packed<Self>,
    ) -> Option<ribbit::Packed<Self>> {
        let parent_len = parent.len().value() as usize;
        let child_len = child.len().value() as usize;
        let len = parent_len + 1 + child_len;
        if len > Self::MAX {
            return None;
        }

        let bit = parent_len << 3;
        Some(ribbit::Packed::<Self>::new(
            unsafe {
                u56::new_unchecked(
                    parent
                        .value
                        .value()
                        .bitor((byte as u64) << bit)
                        .bitor(child.value.value() << (bit + 8))
                        .bitand(Self::MASK),
                )
            },
            unsafe { u3::new_unchecked(len as u8) },
        ))
    }

    pub(crate) fn with_bytes<F: FnOnce(&[u8]) -> T, T>(
        key: ribbit::Packed<Self>,
        prefix: Option<u8>,
        with: F,
    ) -> T {
        let bytes = match prefix {
            // Implicitly shift off len
            Some(prefix) => ((key.value.value() << 8) | prefix as u64).to_ne_bytes(),
            None => key.buffer().value().to_ne_bytes(),
        };
        let slice = &bytes[..key.len().value() as usize + prefix.is_some() as usize];
        with(slice)
    }

    pub(crate) fn bytes(&self) -> impl core::iter::Iterator<Item = u8> {
        self.buffer
            .value()
            .to_ne_bytes()
            .into_iter()
            .take(self.len.value() as usize)
    }
}

#[derive(Debug)]
pub(crate) enum Match {
    Full,
    Partial {
        start: ribbit::Packed<Array>,
        middle: u8,
        end: ribbit::Packed<Array>,
    },
}

impl Debug for Array {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.bytes()).finish()
    }
}

pub(crate) trait Iterator: Clone + core::fmt::Debug + Default {
    fn len(&self) -> usize;
    fn take(&mut self, len: u3) -> ribbit::Packed<Array>;
    fn next(&mut self) -> Option<u8>;
}

#[derive(Copy, Clone, Debug, Default)]
pub struct Fixed {
    buffer: u64,
    len: u8,
}

impl From<u8> for Fixed {
    #[inline]
    fn from(value: u8) -> Self {
        Self {
            buffer: value as u64,
            len: 1,
        }
    }
}

impl From<u64> for Fixed {
    #[inline]
    fn from(value: u64) -> Self {
        Self {
            buffer: if cfg!(target_endian = "little") {
                value.swap_bytes()
            } else {
                value
            },
            len: 8,
        }
    }
}

impl Iterator for Fixed {
    #[inline]
    fn len(&self) -> usize {
        self.len as usize
    }

    #[inline]
    fn take(&mut self, len: u3) -> ribbit::Packed<Array> {
        let bit = (len.value() as u64) << 3;
        let array = ribbit::Packed::<Array>::new(
            unsafe { u56::new_unchecked(self.buffer & ((1u64 << bit) - 1)) },
            len,
        );
        self.buffer >>= bit;
        self.len -= len.value();
        array
    }

    #[inline]
    fn next(&mut self) -> Option<u8> {
        let some = self.len > 0;
        let byte = self.buffer as u8;
        self.buffer >>= 8;
        self.len = self.len.saturating_sub(1);
        some.then_some(byte)
    }
}

#[derive(Copy, Clone, Debug)]
pub enum Dynamic<'a> {
    // INVARIANT: `len > 8`
    Large(&'a [u8]),
    Small(Fixed),
}

impl<'a> From<&'a [u8]> for Dynamic<'a> {
    #[inline]
    fn from(buffer: &'a [u8]) -> Self {
        match buffer.len() {
            9.. => Self::Large(buffer),
            len => Self::Small(Fixed {
                buffer: unsafe { buffer.as_ptr().cast::<u64>().read_unaligned() },
                len: len as u8,
            }),
        }
    }
}

impl Default for Dynamic<'_> {
    #[inline]
    fn default() -> Self {
        Self::Small(Fixed::default())
    }
}

impl Iterator for Dynamic<'_> {
    #[inline]
    fn len(&self) -> usize {
        match self {
            Dynamic::Large(large) => large.len(),
            Dynamic::Small(small) => small.len as usize,
        }
    }

    #[inline]
    fn take(&mut self, len: u3) -> ribbit::Packed<Array> {
        match self {
            Dynamic::Large(large) => {
                validate!(large.len() > 8);

                let buffer = unsafe { large.as_ptr().cast::<u64>().read_unaligned() };
                let buffer = buffer & ((1u64 << ((len.value() as u64) << 3)) - 1);
                let array =
                    ribbit::Packed::<Array>::new(unsafe { u56::new_unchecked(buffer) }, len);

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

                *self = Self::Small(Fixed {
                    buffer: buffer >> ((8 - after) << 3),
                    len: after as u8,
                });
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
                if large.len() - 1 > 8 {
                    *self = Self::Large(&large[1..]);
                } else {
                    *self = Self::Small(Fixed {
                        buffer: unsafe { large[1..].as_ptr().cast::<u64>().read_unaligned() },
                        len: 8,
                    })
                }

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

    use super::Dynamic;
    use super::Iterator as _;

    #[test]
    fn dynamic_smoke() {
        take_all(b"0123456789", [1])
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
