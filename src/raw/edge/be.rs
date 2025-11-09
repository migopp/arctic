use core::ops::BitOr as _;
use core::ops::BitXor as _;

use ribbit::u56;
use ribbit::u6;

use crate::raw::edge::Meta;

#[derive(Copy, Clone, ribbit::Pack)]
#[ribbit(size = 64)]
pub struct Be {
    bits: u6,
    value: bool,
    frozen: bool,
    prefix: u56,
}

impl Be {
    const MASK_PREFIX: u64 = !0b1100_0000;

    pub(crate) fn from_u64_truncate(value: u64, bits: usize) -> ribbit::Packed<Self> {
        let mask = !(u64::MAX >> bits);
        validate!(bits <= 56);
        validate_eq!(bits & 0b111, 0);
        unsafe { ribbit::Packed::<Self>::new_unchecked(value & mask | (bits as u64)) }
    }
}

impl BePacked {
    pub(crate) fn raw(self) -> u64 {
        self.value
    }
}

impl Meta for Be {
    const DEFAULT: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(u6::new(0), false, false, u56::new(0));

    fn bits(meta: ribbit::Packed<Self>) -> usize {
        meta.bits().value() as usize
    }

    fn equal(left: ribbit::Packed<Self>, right: ribbit::Packed<Self>) -> bool {
        (left.value ^ right.value) & Self::MASK_PREFIX == 0
    }

    fn cmp(left: ribbit::Packed<Self>, right: ribbit::Packed<Self>) -> core::cmp::Ordering {
        (left.value & Self::MASK_PREFIX).cmp(&(right.value & Self::MASK_PREFIX))
    }

    fn is_value(meta: ribbit::Packed<Self>) -> bool {
        meta.value()
    }

    fn is_frozen(meta: ribbit::Packed<Self>) -> bool {
        meta.frozen()
    }

    fn with_frozen(meta: ribbit::Packed<Self>, frozen: bool) -> ribbit::Packed<Self> {
        meta.with_frozen(frozen)
    }

    fn with_value(meta: ribbit::Packed<Self>, value: bool) -> ribbit::Packed<Self> {
        meta.with_value(value)
    }

    fn expand(
        old: ribbit::Packed<Self>,
        new: ribbit::Packed<Self>,
    ) -> Result<(ribbit::Packed<Self>, u8, ribbit::Packed<Self>), usize> {
        if Self::equal(old, new) {
            return Err(Self::bits(new));
        }

        let bits = Self::bits(old).min(Self::bits(new));

        let bits_start = old
            .value
            .bitxor(new.value)
            .bitor(1u64.rotate_right(1) >> bits)
            .leading_zeros() as usize
            & !0b111usize;

        let bits_middle = bits_start + 8;
        Ok((
            Self::from_u64_truncate(old.value, bits_start).with_value(false),
            old.value.rotate_left(bits_middle as u32) as u8,
            Self::from_u64_truncate(old.value << bits_middle, Self::bits(old) - bits_middle)
                .with_value(old.value()),
        ))
    }

    fn compress(
        parent: ribbit::Packed<Self>,
        byte: u8,
        child: ribbit::Packed<Self>,
    ) -> Option<ribbit::Packed<Self>> {
        todo!()
    }
}
