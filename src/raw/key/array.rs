use core::fmt;

use crate::raw::Key;
use crate::raw::edge;
use crate::raw::edge::Key as _;
use crate::raw::key;
use crate::raw::key::Len as _;
use crate::raw::key::Read as _;

impl<const N: usize> Key for [u8; N] {
    type Read<'k> = key::vec::Reader<'k>;
    type Write = Writer<N>;
    type Borrowed = [u8; N];
    type Edge = edge::Le;
    type Len = key::vec::Len;

    #[inline]
    fn clone_from_borrow(borrow: &Self::Borrowed) -> Self {
        *borrow
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
    fn len(_: &Self::Borrowed) -> Self::Len {
        key::vec::Len(N)
    }
}

impl<'k, const N: usize> From<&'k [u8; N]> for key::vec::Reader<'k> {
    fn from(array: &'k [u8; N]) -> Self {
        key::vec::Reader::from(array.as_slice())
    }
}

#[repr(transparent)]
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Writer<const N: usize>([u8; N]);

impl<const N: usize> Default for Writer<N> {
    fn default() -> Self {
        Self([0; N])
    }
}

impl<'k, const N: usize> key::Write<key::vec::Reader<'k>> for Writer<N> {
    type Len = key::vec::Len;

    #[inline]
    fn new(prefix: key::vec::Reader<'k>, key: ribbit::Packed<edge::Le>) -> (Self, Self::Len) {
        let len = prefix.len() + key.len();
        let mut buffer = [0u8; N];
        buffer[..prefix.len().bytes()].copy_from_slice(prefix.as_ref());
        buffer[prefix.len().bytes()..]
            .iter_mut()
            .zip(key)
            .for_each(|(out, r#in)| {
                *out = r#in;
            });
        (Writer(buffer), len)
    }

    #[inline]
    fn replace(&mut self, start: Self::Len, node: u8, edge: ribbit::Packed<edge::Le>) -> Self::Len {
        self.0[start.bytes()] = node;
        self.0[start.bytes() + 1..]
            .iter_mut()
            .zip(edge)
            .for_each(|(out, r#in)| {
                *out = r#in;
            });
        start + key::vec::Len::BYTE + edge.len()
    }
}

impl<const N: usize> fmt::Debug for Writer<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
