pub mod array;
mod discard;
pub mod int;
pub mod vec;

pub(crate) use discard::Discard;

use core::borrow::Borrow;
use core::fmt;
use core::ops::Add;
use core::ops::AddAssign;
use core::ops::Sub;
use core::ops::SubAssign;

use crate::raw::edge;

pub trait Key: Borrow<Self::Borrowed> {
    type Borrowed: 'static + ?Sized;

    #[expect(private_bounds)]
    type Read<'k>: Read<Edge = Self::Edge, Len = Self::Len> + From<&'k Self::Borrowed>;

    #[expect(private_bounds)]
    type Write: for<'k> Write<Self::Read<'k>>;

    #[expect(private_bounds)]
    type Edge: ribbit::Pack<Packed: edge::Meta>;

    #[expect(private_bounds)]
    type Len: Len<<<ribbit::Packed<Self::Edge> as edge::Meta>::Key as edge::Key>::Len>;

    unsafe fn borrow_writer_unchecked(writer: &Self::Write) -> &Self::Borrowed;

    unsafe fn from_writer_unchecked(writer: Self::Write) -> Self;

    fn clone_from_borrow(borrowed: &Self::Borrowed) -> Self;

    fn len(borrowed: &Self::Borrowed) -> Self::Len;
}

pub(crate) trait Read: Copy + fmt::Debug + Default {
    // Hint for fixed-size keys
    const LEN: Option<Self::Len>;

    type Edge: ribbit::Pack<Packed: edge::Meta>;
    type Len: Len<<<ribbit::Packed<Self::Edge> as edge::Meta>::Key as edge::Key>::Len>;

    fn len(&self) -> Self::Len;

    // Linear reads for cursor traversal
    fn next(&mut self) -> Option<u8>;

    #[inline]
    unsafe fn next_unchecked(&mut self) -> u8 {
        match self.next() {
            Some(byte) => byte,
            None if cfg!(feature = "validate") => unreachable!(),
            None => unsafe { core::hint::unreachable_unchecked() },
        }
    }

    fn read(
        &mut self,
        len: <<<Self::Edge as ribbit::Pack>::Packed as edge::Meta>::Key as edge::Key>::Len,
    ) -> <<Self::Edge as ribbit::Pack>::Packed as edge::Meta>::Key;

    #[inline]
    fn match_exact(
        &mut self,
        edge: <Self::Edge as ribbit::Pack>::Packed,
    ) -> Option<<<<Self::Edge as ribbit::Pack>::Packed as edge::Meta>::Key as edge::Key>::Len> {
        let key = edge::Meta::key(edge);
        let len = edge::Key::len(key);
        (self.read(len) == key).then_some(len)
    }

    #[inline]
    fn match_inexact(
        &mut self,
        edge: <Self::Edge as ribbit::Pack>::Packed,
    ) -> (
        <<Self::Edge as ribbit::Pack>::Packed as edge::Meta>::Key,
        bool,
    ) {
        let key = edge::Meta::key(edge);
        let len = edge::Key::len(key);
        let read = self.read(len);
        (read, read == key)
    }

    fn trim(&mut self, len: Self::Len);

    // Prefix operations for prefix and range iteration
    fn prefix(self, end: Self::Len) -> Self;
    fn suffix(self, start: Self::Len) -> Self;
    fn common_prefix(self, other: Self) -> Self;
}

pub(crate) trait Write<R: Read>: fmt::Debug + Default {
    type Len: Copy;

    fn new(prefix: R, key: <ribbit::Packed<R::Edge> as edge::Meta>::Key) -> (Self, Self::Len);

    /// Replace bytes starting at `start` with bytes from `node` and `edge`
    fn replace(&mut self, start: Self::Len, node: u8, edge: ribbit::Packed<R::Edge>) -> Self::Len;
}

pub trait Len<L: edge::Len>:
    Sized
    + Copy
    + AddAssign
    + Add<L, Output = Self>
    + SubAssign
    + Sub<L, Output = Self>
    + PartialOrd<L>
    + PartialOrd
{
    const ZERO: Self;
    const BYTE: Self;

    fn bits(self) -> usize;
    fn bytes(self) -> usize;
}

#[cfg(test)]
mod tests {
    use crate::raw::Key;
    use crate::raw::edge;
    use crate::raw::edge::Len as _;
    use crate::raw::key::Len as _;
    use crate::raw::key::Read as _;

    pub(super) fn take_all<K: Key>(array: &[u8], key: &K::Borrowed, lens: &[usize]) {
        let mut reader = K::Read::from(key);
        let mut index = 0;
        let mut actual = Vec::new();

        for len in
            lens.iter().copied().map(|bytes| bytes << 3).map(
                <<<K::Edge as ribbit::Pack>::Packed as edge::Meta>::Key as edge::Key>::Len::new,
            )
        {
            assert_eq!(reader.len().bytes(), array.len() - index);

            let bytes = len.bits() >> 3;

            actual.clear();
            actual.extend(reader.read(len));
            assert_eq!(actual, &array[index..][..bytes]);

            index += bytes;
        }

        assert_eq!(reader.len().bytes(), array.len() - index);
        assert_eq!(reader.next(), array.get(index).copied());
    }
}
