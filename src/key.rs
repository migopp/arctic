use core::fmt::Debug;
use core::iter;

use ribbit::u4;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
#[ribbit::pack(size = 4, debug, eq, ord)]
pub(crate) struct Len(u4);

impl Len {
    pub(crate) const ZERO: Self = Self(u4::new(0));
    pub(crate) const MAX: usize = 8;

    fn from_usize(len: usize) -> Self {
        unsafe { Self(u4::new_unchecked(len.max(8) as u8)) }
    }

    pub(crate) const fn to_usize(self) -> usize {
        self.0.value() as usize
    }
}

#[derive(Copy, Clone, Default, PartialEq, Eq)]
#[ribbit::pack(size = 72)]
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
            .bytes()
            .zip(right.bytes())
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
        parent + 1 + child <= Len::MAX
    }

    pub(crate) fn compress(parent: &Self, byte: u8, child: &Self) -> Self {
        let mut buffer = [0u8; Len::MAX];

        parent
            .bytes()
            .chain(iter::once(byte))
            .chain(child.bytes())
            .zip(&mut buffer)
            .for_each(|(byte, save)| *save = byte);

        Self::from_slice(&buffer[..parent.len.to_usize() + 1 + child.len.to_usize()])
    }

    pub(crate) fn bytes(&self) -> impl Iterator<Item = u8> {
        self.buffer
            .to_bytes()
            .into_iter()
            .take(self.len.0.value() as usize)
    }
}

impl Debug for Array {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.bytes()).finish()
    }
}

#[repr(transparent)]
#[derive(Copy, Clone, Default, PartialEq, Eq)]
#[ribbit::pack(size = 64)]
struct Buffer(u64);

impl Buffer {
    fn from_slice(key: &[u8]) -> (Self, Len) {
        let mut buffer = [0u8; Len::MAX];
        let len = key.len().min(Len::MAX);

        #[cfg(feature = "opt-memcpy")]
        unsafe {
            #[cfg(not(target_arch = "x86_64"))]
            compile_error!("opt-memcpy requires target_arch=x86_64");

            core::arch::asm! {
                "mov rcx, {len}",
                "mov rsi, {input}",
                "mov rdi, {output}",
                "rep movsb",
                len = in(reg) len,
                input = in(reg) key.as_ptr(),
                output = in(reg) &mut buffer,
                out("rcx") _,
                out("rsi") _,
                out("rdi") _,
                options(nostack),
            }
        }

        #[cfg(not(feature = "opt-memcpy"))]
        buffer[..len].copy_from_slice(&key[..len]);

        (Self(u64::from_ne_bytes(buffer)), unsafe {
            Len(u4::new_unchecked(len as u8))
        })
    }

    fn to_bytes(self) -> [u8; 8] {
        self.0.to_ne_bytes()
    }
}
