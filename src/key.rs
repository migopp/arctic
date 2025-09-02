use core::iter;

use ribbit::u4;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
#[ribbit::pack(size = 4, debug, eq, ord)]
pub(crate) struct Len(u4);

impl Len {
    pub(crate) const ZERO: Self = Self(u4::new(0));
    pub(crate) const MAX: Self = Self(u4::new(8));

    const fn from_usize(len: usize) -> Self {
        if len > Self::MAX.to_usize() {
            Self::MAX
        } else {
            unsafe { Self(u4::new_unchecked(len as u8)) }
        }
    }

    pub(crate) const fn to_usize(self) -> usize {
        self.0.value() as usize
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[ribbit::pack(size = 72, debug)]
pub(crate) struct Array {
    #[ribbit(size = 64)]
    buffer: Buffer,

    #[ribbit(size = 4)]
    pub(crate) len: Len,
}

impl Array {
    pub(crate) fn from_slice(key: &[u8]) -> Self {
        let (buffer, len) = Buffer::from_slice(key);
        Self { buffer, len }
    }

    pub(crate) fn prefix(left: &Self, right: &Self) -> Len {
        let len = left
            .iter()
            .zip(right.iter())
            .take_while(|(l, r)| l == r)
            .count();

        Len(u4::new(len as u8))
    }

    pub(crate) fn expand(&self, index: Len) -> (Self, u8, Self) {
        let buffer = self.buffer.to_bytes();
        let index = index.to_usize();
        let len = self.len.to_usize();

        (
            Self::from_slice(&buffer[..index]),
            buffer[index],
            Self::from_slice(&buffer[index + 1..len]),
        )
    }

    pub(crate) fn can_compress(parent: &Self, child: &Self) -> bool {
        let parent = parent.len.to_usize();
        let child = child.len.to_usize();
        parent + 1 + child <= Len::MAX.to_usize()
    }

    pub(crate) fn compress(parent: &Self, byte: u8, child: &Self) -> Self {
        let mut buffer = [0u8; Len::MAX.to_usize()];

        parent
            .iter()
            .chain(iter::once(byte))
            .chain(child.iter())
            .zip(&mut buffer)
            .for_each(|(byte, save)| *save = byte);

        Self::from_slice(&buffer[..parent.len.to_usize() + 1 + child.len.to_usize()])
    }

    fn iter(&self) -> impl Iterator<Item = u8> {
        self.buffer
            .to_bytes()
            .into_iter()
            .take(self.len.0.value() as usize)
    }
}

#[repr(transparent)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[ribbit::pack(size = 64, debug, eq)]
struct Buffer(u64);

impl Buffer {
    fn from_slice(key: &[u8]) -> (Self, Len) {
        let len = Len::from_usize(key.len());
        let mut buffer = [0u8; Len::MAX.to_usize()];
        buffer[..len.to_usize()].copy_from_slice(&key[..len.to_usize()]);
        (Self(u64::from_be_bytes(buffer)), len)
    }

    fn to_bytes(self) -> [u8; 8] {
        self.0.to_be_bytes()
    }
}
