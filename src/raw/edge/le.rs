use core::ops::BitOr as _;

use ribbit::u3;
use ribbit::u6;
use ribbit::u56;

use crate::raw::edge;

#[derive(Copy, Clone, Debug, ribbit::Pack)]
#[ribbit(size = 64, debug)]
pub struct Le {
    prefix: u56,
    value: bool,
    frozen: bool,
    #[ribbit(offset = 59)]
    len: u3,
}

impl Le {
    const MASK_FLAG: u64 = 0b0000_0111u64 << 56;
    const MASK_LEN: u64 = 0b0011_1000 << 56;
    const MASK_PREFIX: u64 = !(0b1111_1111 << 56);

    #[inline]
    pub(crate) fn new(value: u64, len: u6) -> ribbit::Packed<Self> {
        validate_eq!(len.value() & 0b111, 0);
        unsafe {
            ribbit::Packed::<Self>::new_unchecked(
                value & Self::MASK_PREFIX | ((len.value() as u64) << 56),
            )
        }
    }
}

impl LePacked {
    #[inline]
    pub(crate) fn raw(self) -> u64 {
        self.value
    }
}

impl IntoIterator for LePacked {
    type Item = u8;
    type IntoIter = core::iter::Take<core::array::IntoIter<u8, 8>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.value
            .to_le_bytes()
            .into_iter()
            .take(self.len().value() as usize)
    }
}

impl edge::Meta for LePacked {
    const DEFAULT: Self = Self::new(u56::new(0), false, false, u3::new(0));

    type Len = u6;

    #[inline]
    fn len(self) -> u6 {
        unsafe { u6::new_unchecked(((self.value & Le::MASK_LEN) >> 56) as u8) }
    }

    #[inline]
    fn is_value(self) -> bool {
        self.value()
    }

    #[inline]
    fn is_frozen(self) -> bool {
        self.frozen()
    }

    #[inline]
    fn is_less_than(self, rhs: Self) -> bool {
        self.value.swap_bytes() < rhs.value.swap_bytes()
    }

    #[inline]
    fn with_value(self, value: bool) -> Self {
        self.with_value(value)
    }

    #[inline]
    fn with_frozen(self, frozen: bool) -> Self {
        self.with_frozen(frozen)
    }

    #[inline]
    fn with_key(self, key: Self) -> Self {
        validate_eq!(key.value & Le::MASK_FLAG, 0);
        unsafe { Self::new_unchecked(self.value & Le::MASK_FLAG | key.value) }
    }

    // #[inline]
    // fn expand(self, new: Self::Key) -> Result<(Self, u8, Self), ()> {
    //     if self.key() == new {
    //         return Err(());
    //     }
    //
    //     if cfg!(feature = "opt-no-expand") {
    //         let len = (new.len().value() >> 3) as usize;
    //         let len_parent = self
    //             .value
    //             .to_le_bytes()
    //             .into_iter()
    //             .zip(new.value.to_le_bytes())
    //             .take(len)
    //             .position(|(l, r)| l != r)
    //             .unwrap_or(len);
    //
    //         let bytes = self.value.to_le_bytes();
    //         let mut parent = [0u8; 8];
    //         parent[..len_parent].copy_from_slice(&bytes[..len_parent]);
    //         let parent = Le::key_from_u64_truncate(
    //             u64::from_le_bytes(parent),
    //             u6::new((len_parent << 3) as u8),
    //         );
    //
    //         let middle = bytes[len_parent];
    //
    //         let len_total = (self.len().value() as usize) >> 3;
    //         let len_child = len_total - len_parent - 1;
    //         let mut child = [0u8; 8];
    //         child[..len_child].copy_from_slice(&bytes[len_parent + 1..len_total]);
    //         let child = Le::key_from_u64_truncate(
    //             u64::from_le_bytes(child),
    //             u6::new((len_child << 3) as u8),
    //         )
    //         .with_meta(self);
    //
    //         return Ok((parent, middle, child));
    //     }
    //
    //     validate!(self.len() >= new.len());
    //
    //     let len_parent = unsafe {
    //         u6::new_unchecked(
    //             self.value
    //                 .bitxor(new.value)
    //                 .bitor(1u64 << new.len().value())
    //                 .trailing_zeros() as u8
    //                 & !0b111u8,
    //         )
    //     };
    //
    //     let len_middle = unsafe { u6::new_unchecked(len_parent.value() + 8) };
    //
    //     Ok((
    //         Le::key_from_u64_truncate(self.value, len_parent).with_value(false),
    //         (self.value >> len_parent.value()) as u8,
    //         Le::key_from_u64_truncate(self.value >> len_middle.value(), self.len() - len_middle)
    //             .with_meta(self),
    //     ))
    // }

    #[inline]
    fn compress(self, byte: u8, child: Self) -> Option<Self> {
        validate!(self.frozen());

        let parent_bits = ((self.value & Le::MASK_LEN) >> 56) as u8;
        let child_bits = ((child.value & Le::MASK_LEN) >> 56) as u8;
        let len = u6::try_new(parent_bits + 8 + child_bits).ok()?;

        Some(
            Le::new(
                (self.value & ((1 << parent_bits) - 1))
                    .bitor((byte as u64) << parent_bits)
                    .bitor(child.value << (parent_bits + 8)),
                len,
            )
            .with_value(child.value()),
        )
    }
}
