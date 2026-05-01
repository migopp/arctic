use core::ops::Add;
use core::ops::AddAssign;
use core::ops::Sub;
use core::ops::SubAssign;

use ribbit::u6;

use crate::raw::Int;
use crate::raw::Key;
use crate::raw::edge;
use crate::raw::edge::Key as _;
use crate::raw::key;
use crate::raw::key::Len as _;
use crate::raw::key::Read as _;

macro_rules! impl_key {
    ($($ty:ty),* $(,)?) => {
        $(
            impl Key for $ty {
                type Read<'k> = Reader<$ty>;
                type Write = Writer<$ty>;
                type Borrowed = Self;

                type Edge = edge::Be;
                type Len = Len;

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
                    Len(<$ty as Int>::BITS)
                }
            }

            impl From<$ty> for Reader<$ty> {
                #[inline]
                fn from(value: $ty) -> Self {
                    Self {
                        buffer: value,
                        len: Len(<$ty as Int>::BITS),
                    }
                }
            }

            impl<'k> From<&'k $ty> for Reader<$ty> {
                #[inline]
                fn from(value: &'k $ty) -> Self {
                    Self::from(*value)
                }
            }
        )*
    };
}

impl_key!(u16, u32, u128);

#[cfg(not(feature = "opt-no-int"))]
impl_key!(u64);

#[derive(Copy, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Reader<I> {
    pub(crate) buffer: I,
    len: Len,
}

#[expect(private_bounds)]
impl<I: Int> Reader<I> {
    #[inline]
    pub unsafe fn new_unchecked(buffer: I, bits: u8) -> Self {
        validate!(bits <= I::BITS);
        validate_eq!(bits & 0b111, 0);
        validate_eq!(buffer.most_significant(bits), buffer);
        Self {
            buffer,
            len: Len(bits),
        }
    }
}

impl<I: Int> key::Read for Reader<I> {
    const LEN: Option<Self::Len> = Some(Len(I::BITS));

    type Edge = edge::Be;
    type Len = Len;

    #[inline]
    fn len(&self) -> Self::Len {
        self.len
    }

    #[inline]
    fn next(&mut self) -> Option<u8> {
        (self.len > Len::ZERO).then(|| unsafe { self.next_unchecked() })
    }

    #[inline]
    unsafe fn next_unchecked(&mut self) -> u8 {
        validate!(self.len > Len::ZERO);
        let byte = self.buffer.most_significant_u8();
        self.buffer <<= 8;
        self.len = Len(self.len.0.saturating_sub(8));
        byte
    }

    #[inline]
    fn read(
        &mut self,
        len: <<<Self::Edge as ribbit::Pack>::Packed as edge::Meta>::Key as edge::Key>::Len,
    ) -> <<Self::Edge as ribbit::Pack>::Packed as edge::Meta>::Key {
        let len = edge::Be::min_len(len, self.len.0 as usize);
        let meta = edge::Be::key_from_u64_truncate(self.buffer.most_significant_u64(), len);
        self.buffer = self.buffer.shl_at_most_56(len.value());
        self.len -= len;
        meta
    }

    #[inline]
    fn trim(&mut self, len: Self::Len) {
        self.len -= len;
    }

    #[inline]
    fn prefix(self, end: Self::Len) -> Self {
        validate!(end <= self.len());

        Self {
            buffer: self.buffer,
            len: end,
        }
    }

    #[inline]
    fn suffix(self, start: Self::Len) -> Self {
        validate!(start <= self.len());

        Self {
            buffer: self.buffer.unbounded_shl(start.0),
            len: self.len - start,
        }
    }

    #[inline]
    fn common_prefix(self, other: Self) -> Self {
        let max = self.len.min(other.len).0;
        let len = Len((self.buffer ^ other.buffer).leading_zeros().min(max) & !0b111);
        Self {
            buffer: self.buffer.most_significant(len.0),
            len,
        }
    }
}

impl<I: Int> core::fmt::Debug for Reader<I> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let bytes = self.len().bytes();
        self.buffer
            .with_be_bytes(|buffer| f.debug_list().entries(&buffer[..bytes]).finish())
    }
}

#[repr(transparent)]
#[derive(Default)]
pub struct Writer<I>(I);

impl<I: Int> key::Write<Reader<I>> for Writer<I> {
    type Len = Len;

    #[inline]
    fn new(prefix: Reader<I>, key: ribbit::Packed<edge::Be>) -> (Self, Self::Len) {
        let len = prefix.len() + key.len();

        validate!(len.0 <= I::BITS);

        let writer = Self(prefix.buffer | I::from_most_significant_u64(key.raw()) >> prefix.len.0);

        (writer, len)
    }

    #[inline]
    fn replace(&mut self, start: Self::Len, node: u8, edge: ribbit::Packed<edge::Be>) -> Self::Len {
        self.0 = self.0.most_significant(start.0)
            | (I::from_u8(node) >> start.0)
            | (I::from_most_significant_u64(edge.raw() & !0xFFu64).unbounded_shr(8 + start.0));

        start + Len::BYTE + edge.len()
    }
}

impl<I: Int> core::fmt::Debug for Writer<I> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0
            .with_be_bytes(|bytes| f.debug_list().entries(bytes).finish())
    }
}

#[derive(Copy, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Len(u8);

impl key::Len<u6> for Len {
    const ZERO: Self = Self(0);
    const BYTE: Self = Self(8);

    #[inline]
    fn bits(self) -> usize {
        self.0 as usize
    }

    #[inline]
    fn bytes(self) -> usize {
        (self.0 >> 3) as usize
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
        Self(self.0 + rhs.value())
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
        self.0 -= rhs.value();
    }
}

impl Sub<u6> for Len {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: u6) -> Self::Output {
        Self(self.0 - rhs.value())
    }
}

impl PartialOrd<u6> for Len {
    #[inline]
    fn partial_cmp(&self, other: &u6) -> Option<std::cmp::Ordering> {
        Some(self.0.cmp(&other.value()))
    }
}

impl PartialEq<u6> for Len {
    #[inline]
    fn eq(&self, other: &u6) -> bool {
        self.0.eq(&other.value())
    }
}

#[cfg(test)]
mod tests {
    use crate::raw::key::tests::take_all;

    #[test]
    fn smoke() {
        take_all_u64(0x1234_5678_9ABC_DEF0u64, &[7, 1]);
    }

    #[test]
    fn take_0() {
        take_all_u64(0x1234_5678_9ABC_DEF0u64, &[0, 1, 0]);
    }

    #[test]
    fn take_1() {
        take_all_u64(0x1234_5678_9ABC_DEF0u64, &[1, 1, 1, 1, 1, 1, 1, 1]);
    }

    fn take_all_u64(key: u64, lens: &[usize]) {
        take_all::<u64>(key.to_be_bytes().as_slice(), &key, lens)
    }
}
