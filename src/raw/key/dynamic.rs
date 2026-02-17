use core::fmt;

use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key;

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Reader<'k>(&'k [u8]);

impl<'k> AsRef<[u8]> for Reader<'k> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.0
    }
}

impl<'k> From<&'k [u8]> for Reader<'k> {
    #[inline]
    fn from(key: &'k [u8]) -> Self {
        Self(key)
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
        Self(&[])
    }
}

impl key::Read for Reader<'_> {
    const BITS: Option<usize> = None;

    type Edge = edge::Le;

    #[inline]
    fn bits(&self) -> usize {
        self.0.len() << 3
    }

    #[inline]
    fn next(&mut self) -> Option<u8> {
        self.0.split_off_first().copied()
    }

    #[inline]
    fn read(
        &mut self,
        len: <<<Self::Edge as ribbit::Pack>::Packed as edge::Meta>::Key as edge::Key>::Len,
    ) -> ribbit::Packed<edge::Le> {
        if len.bits() == 0 {
            return ribbit::Packed::<edge::Le>::DEFAULT;
        }

        let len = edge::Le::min_len(len, self.0.len() << 3);

        let buffer = if self.0.len() >= 8 {
            unsafe { self.0.as_ptr().cast::<u64>().read_unaligned() }
        } else {
            let mut buffer = [0u8; 8];
            buffer[..self.0.len()].copy_from_slice(self.0);
            u64::from_le_bytes(buffer)
        };

        self.0 = &self.0[len.bits() >> 3..];
        edge::Le::key_from_u64_truncate(buffer, len)
    }

    #[inline]
    fn prefix(self, bits: usize) -> Self {
        validate!(self.bits() >= bits);
        Reader(&self.0[..bits >> 3])
    }

    #[inline]
    fn suffix(self, bits: usize) -> Self {
        validate!(self.bits() >= bits);
        Self(&self.0[bits >> 3..])
    }

    #[inline]
    fn common_prefix(self, other: Self) -> Self {
        let index = core::iter::zip(self.0, other.0)
            .position(|(l, r)| l != r)
            .unwrap_or_else(|| self.0.len().min(other.0.len()));
        Self(&self.0[..index])
    }
}

#[repr(transparent)]
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Writer(pub(super) Vec<u8>);

impl key::Write for Writer {
    type Edge = edge::Le;
    type Len = usize;

    #[inline]
    fn len_from_bits(bits: usize) -> Self::Len {
        bits >> 3
    }

    #[inline]
    fn write(&mut self, len: Self::Len, edge: ribbit::Packed<Self::Edge>) -> Self::Len {
        validate_eq!(len, self.0.len());
        self.0.extend(edge);
        self.0.len()
    }

    #[inline]
    fn replace(
        &mut self,
        start: Self::Len,
        node: u8,
        edge: ribbit::Packed<Self::Edge>,
    ) -> Self::Len {
        validate!(start <= self.0.len());
        self.0.truncate(start);
        self.0.push(node);
        self.0.extend(edge);
        self.0.len()
    }
}

impl fmt::Debug for Writer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<'k> From<Reader<'k>> for Writer {
    #[inline]
    fn from(reader: Reader<'k>) -> Self {
        Self(reader.0.to_vec())
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

    fn take_all_array(key: &[u8], lens: &[usize]) {
        take_all::<Vec<u8>>(key, key, lens)
    }
}
