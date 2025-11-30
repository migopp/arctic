#![expect(dead_code)]

use core::ops::BitOr as _;
use core::ops::BitXor;

use ribbit::u56;
use ribbit::u6;

use crate::raw::edge;

#[derive(Copy, Clone, Debug, ribbit::Pack)]
#[ribbit(size = 64, debug, eq, ord)]
pub struct Le {
    prefix: u56,
    value: bool,
    frozen: bool,
    len: u6,
}

impl Le {
    const MASK_META: u64 = 0b11u64 << 56;
    const MASK_KEY: u64 = !Self::MASK_META;

    #[inline]
    pub(crate) fn key_from_u64_truncate(value: u64, len: u6) -> ribbit::Packed<Self> {
        validate_eq!(len.value() & 0b111, 0);
        let mask = (1u64 << len.value()) - 1;
        ribbit::Packed::<Self>::new(
            unsafe { u56::new_unchecked(value & mask) },
            false,
            false,
            len,
        )
    }

    #[inline]
    pub(crate) fn min_len(len: u6, bits: usize) -> u6 {
        unsafe { u6::new_unchecked((len.value() as usize).min(bits) as u8) }
    }
}

impl LePacked {
    pub(crate) fn raw(self) -> u64 {
        self.value
    }
}

impl IntoIterator for LePacked {
    type Item = u8;
    type IntoIter = core::iter::Take<core::array::IntoIter<u8, 8>>;
    fn into_iter(self) -> Self::IntoIter {
        self.value
            .to_le_bytes()
            .into_iter()
            .take((self.len().value() >> 3) as usize)
    }
}

impl edge::Meta for LePacked {
    const DEFAULT: Self = Self::new(u56::new(0), false, false, u6::new(0));
    const MAX_LEN: Self::Len = u6::new(56);

    type Len = u6;
    type Key = Self;

    #[inline]
    fn key(self) -> Self::Key {
        unsafe { Self::new_unchecked(self.value & Le::MASK_KEY) }
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
    fn with_frozen(self, frozen: bool) -> Self {
        self.with_frozen(frozen)
    }

    #[inline]
    fn expand(self, new: Self::Key) -> Result<(Self, u8, Self), ()> {
        if self.key() == new {
            return Err(());
        }

        let len = self.len().min(new.len());

        let len_start = unsafe {
            u6::new_unchecked(
                self.value
                    .bitxor(new.value)
                    .bitor(1u64 << len.value())
                    .trailing_zeros() as u8
                    & !0b111u8,
            )
        };

        let len_middle = unsafe { u6::new_unchecked(len_start.value() + 8) };

        Ok((
            Le::key_from_u64_truncate(self.value, len_start).with_value(false),
            (self.value >> len_start.value()) as u8,
            Le::key_from_u64_truncate(self.value >> len_middle.value(), self.len() - len_middle)
                .with_value(self.value()),
        ))
    }

    #[inline]
    fn compress(self, _byte: u8, _child: Self) -> Option<Self> {
        todo!()
    }
}

impl edge::Key for LePacked {
    type Meta = ribbit::Packed<Le>;
    type Len = u6;

    #[inline]
    fn len(self) -> Self::Len {
        self.len()
    }

    #[inline]
    fn with_value(self, value: bool) -> Self::Meta {
        self.with_value(value)
    }
}
