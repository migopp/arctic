use core::cmp;
use core::fmt;

use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::key;
use crate::raw::key::integer;

#[derive(Copy, Clone)]
pub enum Reader<'k> {
    // INVARIANT: `len > 8`
    Large(&'k [u8]),
    Small(integer::Reader<u64>),
}

impl<'k> From<&'k [u8]> for Reader<'k> {
    #[inline]
    fn from(key: &'k [u8]) -> Self {
        match key.len() {
            9.. => Self::Large(key),
            len => {
                let mut buffer = [0u8; 8];
                buffer[..len].copy_from_slice(key);
                Self::Small(unsafe {
                    integer::Reader::new_unchecked(u64::from_be_bytes(buffer), (len << 3) as u8)
                })
            }
        }
    }
}

impl<'k, const N: usize> From<&'k [u8; N]> for Reader<'k> {
    #[inline]
    fn from(value: &'k [u8; N]) -> Self {
        Self::from(value.as_slice())
    }
}

impl<'k> From<&'k str> for Reader<'k> {
    #[inline]
    fn from(value: &'k str) -> Self {
        Self::from(value.as_bytes())
    }
}

impl Default for Reader<'_> {
    #[inline]
    fn default() -> Self {
        Self::Small(integer::Reader::default())
    }
}

impl key::Read for Reader<'_> {
    const BITS: Option<usize> = None;

    type Edge = edge::Be;

    #[inline]
    fn bits(&self) -> usize {
        match self {
            Reader::Large(large) => large.len() << 3,
            Reader::Small(small) => key::Read::bits(small),
        }
    }

    #[inline]
    fn next(&mut self) -> Option<u8> {
        match self {
            Self::Large(_) => Some(unsafe { self.next_unchecked() }),
            Self::Small(small) => small.next(),
        }
    }

    #[inline]
    unsafe fn next_unchecked(&mut self) -> u8 {
        match self {
            Reader::Large(large) => {
                validate!(large.len() > 8);

                let len = large.len();
                let byte = large[0];

                *self = Self::Large(&large[1..]);
                if len == 9 {
                    *self = Self::Small(self.to_small());
                }
                byte
            }
            Reader::Small(small) => small.next_unchecked(),
        }
    }

    #[inline]
    fn read(
        &mut self,
        len: <<Self::Edge as ribbit::Pack>::Packed as edge::Meta>::Len,
    ) -> ribbit::Packed<Self::Edge> {
        match self {
            Reader::Large(large) => {
                validate!(large.len() > 8);

                let buffer = unsafe { large.as_ptr().cast::<u64>().read_unaligned() };
                let array = edge::Be::key_from_u64_truncate(buffer.to_be(), len);
                let after = (large.len() << 3) - len.bits();

                if after > 64 {
                    let bytes = len.bits() >> 3;
                    *self = Self::Large(&large[bytes..]);
                    return array;
                }

                let buffer = unsafe {
                    (&large[large.len() - 8] as *const u8)
                        .cast::<u64>()
                        .read_unaligned()
                }
                .to_be()
                    << (64 - after);

                *self = Self::Small(unsafe { integer::Reader::new_unchecked(buffer, after as u8) });
                array
            }
            Reader::Small(small) => small.read(len),
        }
    }

    #[inline]
    fn prefix(self, bits: usize) -> Self {
        match self {
            Reader::Large(large) => Reader::from(&large[..bits >> 3]),
            Reader::Small(small) => Reader::Small(small.prefix(bits)),
        }
    }

    fn suffix(self, bits: usize) -> Self {
        validate!(self.bits() >= bits);

        match self {
            Self::Large(large) => Self::from(&large[bits >> 3..]),
            Self::Small(small) => Self::Small(small.suffix(bits)),
        }
    }

    fn common_prefix(self, other: Self) -> Self {
        if let (Self::Large(left), Self::Large(right)) = (self, other) {
            let index = core::iter::zip(left, right)
                .position(|(l, r)| l != r)
                .unwrap_or_else(|| left.len().min(right.len()));
            return Self::from(&left[index..]);
        };

        let left = self.to_small();
        let right = other.to_small();
        Self::Small(left.common_prefix(right))
    }
}

impl Reader<'_> {
    fn to_small(self) -> integer::Reader<u64> {
        match self {
            Reader::Small(small) => small,
            Reader::Large(large) => {
                // SAFETY: `large.len() > 8`
                let buffer = unsafe { large.as_ptr().cast::<u64>().read_unaligned() }.to_be();
                unsafe { integer::Reader::new_unchecked(buffer, 64) }
            }
        }
    }
}

impl Eq for Reader<'_> {}
impl PartialEq for Reader<'_> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Large(left), Self::Large(right)) => left == right,
            (Self::Small(left), Self::Small(right)) => left == right,
            _ => false,
        }
    }
}

impl PartialOrd for Reader<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Reader<'_> {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        match (self, other) {
            (Self::Large(left), Self::Large(right)) => left.cmp(right),
            (Self::Small(left), Self::Small(right)) => left.cmp(right),
            (left, right) => left
                .to_small()
                .cmp(&right.to_small())
                .then_with(|| matches!(left, Self::Large(_)).cmp(&matches!(right, Self::Large(_)))),
        }
    }
}

impl fmt::Debug for Reader<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reader::Large(large) => f.debug_list().entries(*large).finish(),
            Reader::Small(small) => small.fmt(f),
        }
    }
}

#[repr(transparent)]
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Writer(pub(super) Vec<u8>);

impl key::Write for Writer {
    type Edge = edge::Be;
    type Len = usize;

    #[inline]
    fn len_from_bits(bits: usize) -> Self::Len {
        bits
    }

    #[inline]
    fn write(&mut self, bits: Self::Len, edge: ribbit::Packed<Self::Edge>) -> Self::Len {
        validate_eq!(bits, self.0.len() << 3);
        self.0.extend(edge);
        self.0.len() << 3
    }

    fn replace(
        &mut self,
        start: Self::Len,
        node: u8,
        edge: ribbit::Packed<Self::Edge>,
    ) -> Self::Len {
        validate!(start <= (self.0.len() << 3));
        self.0.truncate(start >> 3);
        self.0.push(node);
        self.0.extend(edge);
        self.0.len() << 3
    }
}

impl fmt::Debug for Writer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<'k> From<Reader<'k>> for Writer {
    fn from(iter: Reader<'k>) -> Self {
        Self(match iter {
            Reader::Large(large) => large.to_vec(),
            Reader::Small(small) => small.with_bytes(|small| small.to_vec()),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::raw::key::tests::take_all;

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
