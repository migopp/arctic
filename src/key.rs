use core::fmt::Debug;
use core::ops::BitOr as _;
use core::ops::BitXor as _;
use core::ops::Shr as _;

use ribbit::u3;
use ribbit::u56;
use ribbit::Unpack as _;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
#[ribbit::pack(size = 3, debug, eq, ord)]
pub(crate) struct Len(u3);

impl Len {
    pub(crate) const MAX: usize = 7;

    pub(crate) const fn to_usize(self) -> usize {
        self.0.value() as usize
    }
}

#[derive(Copy, Clone, Default, PartialEq, Eq)]
#[ribbit::pack(size = 59)]
pub(crate) struct Array {
    #[ribbit(size = 56)]
    buffer: Buffer,

    #[ribbit(size = 3)]
    pub(crate) len: Len,
}

impl Array {
    pub(crate) fn from_slice(key: &[u8]) -> Self {
        const MAX: Len = Len(u3::new(Len::MAX as u8));
        let (buffer, len) = Buffer::from_slice_len(key, MAX);
        Self { buffer, len }
    }

    pub(crate) fn match_prefix(key: &[u8], edge: ribbit::Packed<Self>) -> Option<usize> {
        let len = key.len().min(edge.len().unpack().to_usize());
        (unsafe { Self::copy(key, len) } == edge.value.value()).then_some(len)
    }

    pub(crate) fn match_split(key: &[u8], edge: ribbit::Packed<Self>) -> Match {
        let edge_len = edge.len().unpack().to_usize();
        let edge = edge.value.value();

        let key_len = key.len();
        let len = key_len.min(edge_len);

        let key = unsafe { Self::copy(key, len) };

        if key == edge {
            return Match::Full(len);
        }

        let prefix_byte = key
            .bitxor(edge)
            // Guarantee `trailing_zeros` cannot produce more than `len * 8` bits
            .bitor(1u64 << (len << 3))
            .trailing_zeros()
            .shr(3u32) as u8;

        let prefix_bit = prefix_byte << 3;

        Match::Partial {
            start: unsafe {
                ribbit::Packed::<Self>::new(
                    ribbit::Packed::<super::key::Buffer>::new(u56::new_unchecked(
                        edge & ((1u64 << prefix_bit) - 1u64),
                    )),
                    ribbit::Packed::<Len>::new_unchecked(u3::new_unchecked(prefix_byte)),
                )
            },
            middle: (edge >> prefix_bit) as u8,
            end: unsafe {
                ribbit::Packed::<Self>::new(
                    ribbit::Packed::<super::key::Buffer>::new(u56::new_unchecked(
                        (edge & (0x00FF_FFFF_FFFF_FFFF)) >> (prefix_bit + 8),
                    )),
                    ribbit::Packed::<Len>::new_unchecked(u3::new_unchecked(
                        edge_len as u8 - prefix_byte - 1,
                    )),
                )
            },
        }
    }

    // SAFETY: caller must ensure `len <= Len::MAX`
    unsafe fn copy(key: &[u8], len: usize) -> u64 {
        #[repr(C, align(8))]
        union Buffer {
            byte: [u8; 8],
            word: u64,
        }

        const {
            assert!(core::mem::size_of::<Buffer>() == 8);
            assert!(core::mem::align_of::<Buffer>() == 8);
        }

        let mut buffer = Buffer { byte: [0u8; 8] };
        unsafe {
            core::hint::assert_unchecked(len <= Len::MAX);
            buffer.byte[..len].copy_from_slice(&key[..len]);
            buffer.byte[7] = len as u8;
            buffer.word
        }
    }

    pub(crate) fn can_compress(parent: &Self, child: &Self) -> bool {
        let parent = parent.len.to_usize();
        let child = child.len.to_usize();
        parent + 1 + child <= Len::MAX
    }

    /// SAFETY: caller must guarantee `can_compress(parent, child)`.
    pub(crate) unsafe fn compress(parent: &Self, byte: u8, child: &Self) -> Self {
        let index_bit = parent.len.0.value() << 3;
        Self {
            buffer: Buffer(unsafe {
                u56::new_unchecked(
                    parent.buffer.0.value()
                        | ((byte as u64) << index_bit)
                        | (child.buffer.0.value() << (index_bit + 8)),
                )
            }),
            len: Len(unsafe { u3::new_unchecked(parent.len.0.value() + 1 + child.len.0.value()) }),
        }
    }

    pub(crate) fn with_bytes<F: FnOnce(&[u8]) -> T, T>(&self, prefix: Option<u8>, with: F) -> T {
        let bytes = match prefix {
            Some(prefix) => (self.buffer.0.value() << 8 | prefix as u64).to_ne_bytes(),
            None => self.buffer.0.value().to_ne_bytes(),
        };
        let slice = &bytes[..self.len.to_usize() + prefix.is_some() as usize];
        with(slice)
    }

    pub(crate) fn bytes(&self) -> impl Iterator<Item = u8> {
        self.buffer
            .0
            .value()
            .to_ne_bytes()
            .into_iter()
            .take(self.len.0.value() as usize)
    }
}

pub(crate) enum Match {
    Full(usize),
    Partial {
        start: ribbit::Packed<Array>,
        middle: u8,
        end: ribbit::Packed<Array>,
    },
}

impl Debug for Array {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.bytes()).finish()
    }
}

#[repr(transparent)]
#[derive(Copy, Clone, Default, PartialEq, Eq)]
#[ribbit::pack(size = 56)]
struct Buffer(u56);

impl Buffer {
    fn from_slice_len(key: &[u8], len: Len) -> (Self, Len) {
        let mut buffer = [0u8; 8];
        let len = key.len().min(len.to_usize()) & Len::MAX;

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

        (
            Self(unsafe { u56::new_unchecked(u64::from_ne_bytes(buffer)) }),
            unsafe { Len(u3::new_unchecked(len as u8)) },
        )
    }
}
