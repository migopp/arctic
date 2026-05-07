use ribbit::u6;

use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
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

    #[inline]
    fn len(&self) -> Self::Len {
        self.len
    }

    #[inline]
    fn get_edge(
        &self,
        len: <ribbit::Packed<Self::Edge> as edge::Meta>::Len,
    ) -> ribbit::Packed<Self::Edge> {
        let len = u6::new((self.len().bits()).min(len.bits()) as u8);
        edge::Le::new(u64::from_le_bytes(self.buffer), u6::new(len.bits() as u8))
    }

    #[inline]
    fn get_byte(&self, index: u6) -> Option<u8> {
        self.buffer.get(index.bytes()).copied()
    }

    #[inline]
    fn match_prefix(&self, edge: <Self::Edge as ribbit::Pack>::Packed) -> key::vec::Len {
        key::vec::Len(
            self.buffer
                .into_iter()
                .zip(edge)
                .take(self.len.0)
                .position(|(l, r)| l != r)
                .unwrap_or(self.len.0),
        )
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

    fn expand(
        &self,
        edge: ribbit::Packed<Self::Edge>,
    ) -> Result<
        (
            ribbit::Packed<Self::Edge>,
            u8,
            u8,
            ribbit::Packed<Self::Edge>,
        ),
        (),
    > {
        let len_match = self.match_prefix(edge);
        if len_match >= edge.len().into() {
            return Err(());
        }

        validate!(self.len > len_match);
        let len_start = u6::new(len_match.bits() as u8);
        let len_middle = len_start + const { u6::new(8) };
        let len_end = u6::new((self.len().bits() - len_middle.bits()) as u8);

        let mut start = [0u8; 8];
        start[..len_start.bytes()].copy_from_slice(&self.buffer[..len_start.bytes()]);
        let start = edge::Le::new(u64::from_le_bytes(start), len_start);

        let old_middle = (edge.raw() >> len_start.bits()) as u8;
        let new_middle = self.buffer[len_start.bytes()];

        let mut end = [0u8; 8];
        end[..len_end.bytes()].copy_from_slice(&self.buffer[len_middle.bytes()..]);
        let end = edge::Le::new(u64::from_le_bytes(end), len_end);

        Ok((start, old_middle, new_middle, end))
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
