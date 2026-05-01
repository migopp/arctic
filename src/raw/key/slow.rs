use ribbit::u6;

use crate::raw::edge;
use crate::raw::edge::Key as _;
use crate::raw::edge::Len as _;
use crate::raw::key;
use crate::raw::key::Len as _;

#[cfg(feature = "opt-no-int")]
impl Key for u64 {
    type Read<'k> = Reader;
    type Write = key::vec::Writer;
    type Borrowed = Self;
    type Edge = edge::Le;
    type Len = key::vec::Len;

    #[inline]
    fn clone_from_borrow(borrow: &Self::Borrowed) -> Self {
        *borrow
    }

    #[inline]
    unsafe fn borrow_writer_unchecked(_: &Self::Write) -> &Self::Borrowed {
        unimplemented!("Can't get little-endian integer from big-endian slice")
    }

    #[inline]
    unsafe fn from_writer_unchecked(writer: Self::Write) -> Self {
        let buffer: [u8; 8] = writer.0.try_into().unwrap();
        u64::from_be_bytes(buffer)
    }

    #[inline]
    fn len(_: &Self::Borrowed) -> usize {
        (<u64 as crate::raw::Int>::BITS >> 3) as usize
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Reader {
    pub(crate) buffer: [u8; 8],
    len: key::vec::Len,
}

impl Reader {
    #[inline]
    pub unsafe fn new_unchecked(buffer: u64, bits: u8) -> Self {
        validate!(bits <= 64);
        validate_eq!(bits & 0b111, 0);
        validate_eq!(buffer & !u64::MAX.unbounded_shl(bits as u32), buffer);
        let buffer = buffer.to_be_bytes();
        Self {
            buffer,
            len: key::vec::Len((bits as usize) >> 3),
        }
    }
}

impl key::Read for Reader {
    const LEN: Option<Self::Len> = Some(key::vec::Len(8));

    type Edge = edge::Le;
    type Len = key::vec::Len;

    fn len(&self) -> Self::Len {
        self.len
    }

    #[inline]
    fn next(&mut self) -> Option<u8> {
        (self.len > key::vec::Len::ZERO).then(|| unsafe { self.next_unchecked() })
    }

    #[inline]
    unsafe fn next_unchecked(&mut self) -> u8 {
        let byte = self.buffer[0];
        self.buffer.copy_within(1.., 0);
        self.len -= key::vec::Len::BYTE;
        byte
    }

    #[inline]
    fn read(
        &mut self,
        len: <<<Self::Edge as ribbit::Pack>::Packed as edge::Meta>::Key as edge::Key>::Len,
    ) -> <<Self::Edge as ribbit::Pack>::Packed as edge::Meta>::Key {
        let len = self.len.min(key::vec::Len(len.bytes()));
        let key = edge::Le::key_from_u64_truncate(
            u64::from_le_bytes(self.buffer),
            u6::new(len.bits() as u8),
        );
        self.buffer.copy_within(len.0.., 0);
        self.len -= len;
        key
    }

    #[inline]
    fn match_exact(
        &mut self,
        edge: <Self::Edge as ribbit::Pack>::Packed,
    ) -> Option<<<<Self::Edge as ribbit::Pack>::Packed as edge::Meta>::Key as edge::Key>::Len> {
        let (key, exact) = self.match_inexact(edge);
        exact.then_some(key.len())
    }

    #[inline]
    fn match_inexact(
        &mut self,
        edge: <Self::Edge as ribbit::Pack>::Packed,
    ) -> (
        <<Self::Edge as ribbit::Pack>::Packed as edge::Meta>::Key,
        bool,
    ) {
        let len = self.len.min(key::vec::Len(edge.len().bytes()));
        let len_prefix = self
            .buffer
            .into_iter()
            .zip(edge.raw().to_le_bytes())
            .take(len.0)
            .position(|(l, r)| l != r)
            .unwrap_or(len.0);

        let key = edge::Le::key_from_u64_truncate(
            u64::from_le_bytes(self.buffer),
            u6::new(len.bits() as u8),
        );

        self.buffer.copy_within(len.0.., 0);
        self.len -= len;
        (key, len.0 == len_prefix)
    }

    #[inline]
    fn trim(&mut self, len: Self::Len) {
        self.len -= len;
    }

    #[inline]
    fn prefix(self, end: Self::Len) -> Self {
        let mut buffer = [0u8; 8];
        buffer[..end.0].copy_from_slice(&self.buffer[..end.0]);
        Self { buffer, len: end }
    }

    #[inline]
    fn suffix(self, start: Self::Len) -> Self {
        let mut buffer = [0u8; 8];
        let len = self.len - start;
        buffer[..len.0].copy_from_slice(&self.buffer[start.0..]);
        Self { buffer, len }
    }

    #[inline]
    fn common_prefix(self, other: Self) -> Self {
        let len = self.len.min(other.len);
        let len_prefix = self.buffer[..len.0]
            .iter()
            .zip(&other.buffer[..len.0])
            .position(|(l, r)| l != r)
            .map(key::vec::Len)
            .unwrap_or(len);
        let mut buffer = [0u8; 8];
        buffer[..len_prefix.0].copy_from_slice(&self.buffer[..len_prefix.0]);
        Self {
            buffer,
            len: len_prefix,
        }
    }
}

impl From<u64> for Reader {
    fn from(key: u64) -> Self {
        unsafe { Reader::new_unchecked(key, 64) }
    }
}

impl<'k> From<&'k u64> for Reader {
    fn from(key: &'k u64) -> Self {
        unsafe { Reader::new_unchecked(*key, 64) }
    }
}
