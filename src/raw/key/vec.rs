use core::fmt;
use core::ops::Add;
use core::ops::AddAssign;
use core::ops::Sub;
use core::ops::SubAssign;

use ribbit::u6;

use crate::raw::Key;
use crate::raw::edge;
use crate::raw::edge::Key as _;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Read as _;

impl Key for Vec<u8> {
    type Read<'k> = Reader<'k, { usize::MAX }>;
    type Write = Writer;
    type Borrowed = [u8];
    type Edge = edge::Le;
    type Len = Len;

    #[inline]
    fn clone_from_borrow(borrow: &Self::Borrowed) -> Self {
        Vec::from(borrow)
    }

    #[inline]
    unsafe fn borrow_writer_unchecked(writer: &Self::Write) -> &Self::Borrowed {
        &writer.0
    }

    #[inline]
    unsafe fn from_writer_unchecked(writer: Self::Write) -> Self {
        writer.0
    }

    #[inline]
    fn len(slice: &Self::Borrowed) -> Self::Len {
        Len(slice.len())
    }
}

impl Key for String {
    type Read<'k> = Reader<'k, { usize::MAX }>;
    type Write = Writer;
    type Borrowed = str;
    type Edge = edge::Le;
    type Len = Len;

    #[inline]
    fn clone_from_borrow(borrow: &Self::Borrowed) -> Self {
        String::from(borrow)
    }

    #[inline]
    unsafe fn borrow_writer_unchecked(writer: &Self::Write) -> &Self::Borrowed {
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

    #[inline]
    fn len(string: &Self::Borrowed) -> Self::Len {
        Len(string.len())
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Reader<'k, const N: usize>(pub(super) &'k [u8]);

impl<'k, const N: usize> AsRef<[u8]> for Reader<'k, N> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.0
    }
}

impl<'k> From<&'k [u8]> for Reader<'k, { usize::MAX }> {
    #[inline]
    fn from(key: &'k [u8]) -> Self {
        Self(key)
    }
}

impl<'k> From<&'k Vec<u8>> for Reader<'k, { usize::MAX }> {
    #[inline]
    fn from(key: &'k Vec<u8>) -> Self {
        Self(key)
    }
}

impl<'k> From<&'k str> for Reader<'k, { usize::MAX }> {
    #[inline]
    fn from(value: &'k str) -> Self {
        Self(value.as_bytes())
    }
}

impl<'k> From<&'k String> for Reader<'k, { usize::MAX }> {
    #[inline]
    fn from(key: &'k String) -> Self {
        Self(key.as_bytes())
    }
}

impl<const N: usize> Default for Reader<'_, N> {
    #[inline]
    fn default() -> Self {
        Self(&[])
    }
}

impl<const N: usize> key::Read for Reader<'_, N> {
    const LEN: Option<Self::Len> = if N == usize::MAX { None } else { Some(Len(N)) };

    type Edge = edge::Le;
    type Len = Len;

    #[inline]
    fn len(&self) -> Self::Len {
        Len(self.0.len())
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
    fn trim(&mut self, len: Self::Len) {
        self.0 = &self.0[..(self.len() - len).0]
    }

    #[inline]
    fn prefix(self, end: Self::Len) -> Self {
        validate!(end <= self.len());
        Reader(&self.0[..end.0])
    }

    #[inline]
    fn suffix(self, start: Self::Len) -> Self {
        validate!(start <= self.len());
        Self(&self.0[start.0..])
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
#[derive(Default)]
pub struct Writer(pub(super) Vec<u8>);

impl<'k> key::Write<Reader<'k, { usize::MAX }>> for Writer {
    type Len = Len;

    #[inline]
    fn new(prefix: Reader<'k, { usize::MAX }>, key: ribbit::Packed<edge::Le>) -> (Self, Self::Len) {
        let len = prefix.len() + key.len();
        let mut buffer = Vec::new();
        buffer.extend_from_slice(prefix.0);
        buffer.extend(key);
        (Writer(buffer), len)
    }

    #[inline]
    fn replace(&mut self, start: Self::Len, node: u8, edge: ribbit::Packed<edge::Le>) -> Self::Len {
        validate!(start.0 <= self.0.len());
        self.0.truncate(start.0);
        self.0.push(node);
        self.0.extend(edge);
        Len(self.0.len())
    }
}

impl fmt::Debug for Writer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<'k> From<Reader<'k, { usize::MAX }>> for Writer {
    #[inline]
    fn from(reader: Reader<'k, { usize::MAX }>) -> Self {
        Self(reader.0.to_vec())
    }
}

#[derive(Copy, Clone, Default, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Len(pub(super) usize);

impl key::Len<u6> for Len {
    const ZERO: Self = Self(0);
    const BYTE: Self = Self(1);

    #[inline]
    fn bits(self) -> usize {
        self.0 << 3
    }

    #[inline]
    fn bytes(self) -> usize {
        self.0
    }
}

impl Add for Len {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl AddAssign for Len {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl Add<u6> for Len {
    type Output = Self;
    #[inline]
    fn add(self, rhs: u6) -> Self::Output {
        Self(self.0 + rhs.bytes())
    }
}

impl Sub for Len {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl SubAssign for Len {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
    }
}

impl SubAssign<u6> for Len {
    #[inline]
    fn sub_assign(&mut self, rhs: u6) {
        self.0 -= rhs.bytes();
    }
}

impl Sub<u6> for Len {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: u6) -> Self::Output {
        Self(self.0 - rhs.bytes())
    }
}

impl PartialOrd<u6> for Len {
    #[inline]
    fn partial_cmp(&self, other: &u6) -> Option<std::cmp::Ordering> {
        Some(self.0.cmp(&other.bytes()))
    }
}

impl PartialEq<u6> for Len {
    #[inline]
    fn eq(&self, other: &u6) -> bool {
        self.0.eq(&other.bytes())
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
