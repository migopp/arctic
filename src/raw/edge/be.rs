use core::ops::BitOr as _;
use core::ops::BitXor as _;

use ribbit::u56;
use ribbit::u6;

use crate::raw::edge;

#[derive(Copy, Clone, ribbit::Pack)]
#[ribbit(size = 64, eq, ord)]
pub struct Be {
    len: u6,
    value: bool,
    frozen: bool,
    prefix: u56,
}

impl Be {
    const MASK_META: u64 = 0b1100_0000;
    const MASK_KEY: u64 = !Self::MASK_META;

    #[inline]
    pub(crate) fn key_from_u64_truncate(value: u64, len: u6) -> ribbit::Packed<Self> {
        let mask = !(u64::MAX >> len.value());
        validate_eq!(len.value() & 0b111, 0);
        unsafe { ribbit::Packed::<Self>::new_unchecked(value & mask | (len.value() as u64)) }
    }

    #[inline]
    pub(crate) fn min_len(len: u6, bits: usize) -> u6 {
        unsafe { u6::new_unchecked((len.value() as usize).min(bits) as u8) }
    }
}

impl BePacked {
    pub(crate) fn raw(self) -> u64 {
        self.value
    }
}

impl edge::Meta for ribbit::Packed<Be> {
    const DEFAULT: Self = Self::new(u6::new(0), false, false, u56::new(0));
    const MAX_LEN: Self::Len = u6::new(56);

    type Len = u6;
    type Key = Self;

    #[inline]
    fn key(self) -> Self::Key {
        unsafe { Self::new_unchecked(self.value & Be::MASK_KEY) }
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
                    .bitor(1u64.rotate_right(1) >> len.value())
                    .leading_zeros() as u8
                    & !0b111u8,
            )
        };

        let len_middle = unsafe { u6::new_unchecked(len_start.value() + 8) };
        Ok((
            Be::key_from_u64_truncate(self.value, len_start).with_value(false),
            self.value.rotate_left(len_middle.value() as u32) as u8,
            Be::key_from_u64_truncate(self.value << len_middle.value(), self.len() - len_middle)
                .with_value(self.value()),
        ))
    }

    #[inline]
    fn compress(self, _byte: u8, _child: Self) -> Option<Self> {
        todo!()
    }
}

impl edge::Key for ribbit::Packed<Be> {
    type Meta = ribbit::Packed<Be>;
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

impl edge::Len for u6 {
    #[inline]
    fn bits(self) -> usize {
        self.value() as usize
    }
}
