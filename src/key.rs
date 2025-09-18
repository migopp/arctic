use core::fmt::Debug;
use core::ops::BitAnd as _;
use core::ops::BitOr as _;
use core::ops::BitXor as _;
use core::ops::Shr as _;

use ribbit::u3;
use ribbit::u56;
use ribbit::u59;

#[derive(Copy, Clone, Default, PartialEq, Eq)]
#[ribbit::pack(size = 59)]
pub(crate) struct Array {
    #[ribbit(size = 56)]
    buffer: u56,

    #[ribbit(size = 3)]
    pub(crate) len: u3,
}

impl Array {
    pub(crate) const EMPTY: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(u56::new(0), u3::new(0));

    const MASK: u64 = 0x00FF_FFFF_FFFF_FFFF;
    pub(crate) const MAX: usize = 7;

    pub(crate) fn from_slice(key: &[u8]) -> ribbit::Packed<Self> {
        let len = key.len().min(Self::MAX);
        unsafe { ribbit::Packed::<Self>::new_unchecked(u59::new_unchecked(Self::copy(key, len))) }
    }

    pub(crate) fn match_prefix(key: &[u8], edge: ribbit::Packed<Self>) -> Option<usize> {
        let len = key.len().min(edge.len().value() as usize);
        (unsafe { Self::copy(key, len) } == edge.value.value()).then_some(len)
    }

    pub(crate) fn match_split(key: &[u8], edge: ribbit::Packed<Self>) -> Match {
        let edge_len = edge.len().value() as usize;
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
                    u56::new_unchecked(edge & ((1u64 << prefix_bit) - 1u64)),
                    u3::new_unchecked(prefix_byte),
                )
            },
            middle: (edge >> prefix_bit) as u8,
            end: unsafe {
                ribbit::Packed::<Self>::new(
                    u56::new_unchecked((edge & Self::MASK) >> (prefix_bit + 8)),
                    u3::new_unchecked(edge_len as u8 - prefix_byte - 1),
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
            core::hint::assert_unchecked(len <= Self::MAX);
            buffer.byte[..len].copy_from_slice(&key[..len]);
            buffer.byte[7] = len as u8;
            buffer.word
        }
    }

    pub(crate) fn compress(
        parent: ribbit::Packed<Self>,
        byte: u8,
        child: ribbit::Packed<Self>,
    ) -> Option<ribbit::Packed<Self>> {
        let parent_len = parent.len().value() as usize;
        let child_len = child.len().value() as usize;
        let len = parent_len + 1 + child_len;
        if len > Self::MAX {
            return None;
        }

        let bit = parent_len << 3;
        Some(ribbit::Packed::<Self>::new(
            unsafe {
                u56::new_unchecked(
                    parent
                        .value
                        .value()
                        .bitor((byte as u64) << bit)
                        .bitor(child.value.value() << (bit + 8))
                        .bitand(Self::MASK),
                )
            },
            unsafe { u3::new_unchecked(len as u8) },
        ))
    }

    pub(crate) fn with_bytes<F: FnOnce(&[u8]) -> T, T>(&self, prefix: Option<u8>, with: F) -> T {
        let bytes = match prefix {
            Some(prefix) => (self.buffer.value() << 8 | prefix as u64).to_ne_bytes(),
            None => self.buffer.value().to_ne_bytes(),
        };
        let slice = &bytes[..self.len.value() as usize + prefix.is_some() as usize];
        with(slice)
    }

    pub(crate) fn bytes(&self) -> impl Iterator<Item = u8> {
        self.buffer
            .value()
            .to_ne_bytes()
            .into_iter()
            .take(self.len.value() as usize)
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
