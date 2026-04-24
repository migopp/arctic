pub mod integer;
pub mod vec;

use core::borrow::Borrow;
use core::fmt;
use core::marker::PhantomData;

use crate::raw::edge;

pub trait Key: Borrow<Self::Borrowed> {
    type Borrowed: 'static + ?Sized;

    #[expect(private_bounds)]
    type Read<'k>: Read<Edge = Self::Edge> + From<&'k Self::Borrowed>;

    #[expect(private_bounds)]
    type Write: Write<Edge = Self::Edge> + for<'k> From<Self::Read<'k>>;

    #[expect(private_bounds)]
    type Edge: ribbit::Pack<Packed: edge::Meta>;

    unsafe fn borrow_writer_unchecked(writer: &Self::Write) -> &Self::Borrowed;

    unsafe fn from_writer_unchecked(writer: Self::Write) -> Self;

    fn clone_from_borrow(borrow: &Self::Borrowed) -> Self;

    // Key length in bytes
    fn len(borrow: &Self::Borrowed) -> usize;
}

pub(crate) trait Read: Copy + fmt::Debug + Default {
    // Hint for fixed-size keys
    const BITS: Option<usize>;

    type Edge: ribbit::Pack<Packed: edge::Meta>;

    fn bits(&self) -> usize;

    #[inline]
    fn bytes(&self) -> usize {
        self.bits() >> 3
    }

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

    fn trim(&mut self, bits: usize);

    // Prefix operations for prefix and range iteration
    fn prefix(self, bits: usize) -> Self;
    fn suffix(self, bits: usize) -> Self;
    fn common_prefix(self, other: Self) -> Self;
}

pub(crate) trait Write: Clone + fmt::Debug + Default + Ord {
    type Len: Copy;
    type Edge: ribbit::Pack<Packed: edge::Meta>;

    fn len_from_bits(bits: usize) -> Self::Len;

    /// Write bytes starting at `start` with bytes from `edge`
    ///
    /// Caller must ensure `start` is equal to the current length of this writer
    fn write(&mut self, start: Self::Len, edge: ribbit::Packed<Self::Edge>) -> Self::Len;

    /// Replace bytes starting at `start` with bytes from `node` and `edge`
    fn replace(
        &mut self,
        start: Self::Len,
        node: u8,
        edge: ribbit::Packed<Self::Edge>,
    ) -> Self::Len;
}

#[derive(Clone)]
pub(crate) struct Discard<M>(PhantomData<M>);

impl<M> Default for Discard<M> {
    fn default() -> Self {
        Self(PhantomData)
    }
}
impl<M> core::fmt::Debug for Discard<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Ignore")
    }
}
impl<M> PartialEq for Discard<M> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl<M> Eq for Discard<M> {}
impl<M> Ord for Discard<M> {
    fn cmp(&self, _: &Self) -> core::cmp::Ordering {
        core::cmp::Ordering::Equal
    }
}
impl<M> PartialOrd for Discard<M> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<M> Write for Discard<M>
where
    M: ribbit::Pack<Packed: edge::Meta>,
{
    type Len = ();
    type Edge = M;

    #[inline]
    fn len_from_bits(_bits: usize) -> Self::Len {}

    #[inline]
    fn write(&mut self, (): Self::Len, _edge: ribbit::Packed<Self::Edge>) -> Self::Len {}

    #[inline]
    fn replace(&mut self, _start: Self::Len, _node: u8, _edge: ribbit::Packed<Self::Edge>) {}
}

impl<R, M> From<R> for Discard<M>
where
    R: Read<Edge = M>,
{
    fn from(_: R) -> Self {
        Self(PhantomData)
    }
}

#[cfg(test)]
mod tests {
    use crate::raw::Key;
    use crate::raw::edge;
    use crate::raw::edge::Len as _;
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
            assert_eq!(reader.bytes(), array.len() - index);

            let bytes = len.bits() >> 3;

            actual.clear();
            actual.extend(reader.read(len));
            assert_eq!(actual, &array[index..][..bytes]);

            index += bytes;
        }

        assert_eq!(reader.bytes(), array.len() - index);
        assert_eq!(reader.next(), array.get(index).copied());
    }
}
