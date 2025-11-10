use core::ops::BitOr as _;
use core::ops::BitXor as _;

use ribbit::u56;
use ribbit::u6;

use crate::raw::edge::Meta;

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

impl Meta for Be {
    const DEFAULT: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(u6::new(0), false, false, u56::new(0));
    const MAX_LEN: Self::Len = u6::new(56);

    type Len = u6;
    type Key = ribbit::Packed<Self>;

    #[inline]
    fn key(meta: ribbit::Packed<Self>) -> Self::Key {
        unsafe { ribbit::Packed::<Self>::new_unchecked(meta.value & Self::MASK_KEY) }
    }

    #[inline]
    fn len(key: Self::Key) -> Self::Len {
        key.len()
    }

    #[inline]
    fn len_to_bits(len: Self::Len) -> usize {
        len.value() as usize
    }

    #[inline]
    fn is_value(meta: ribbit::Packed<Self>) -> bool {
        meta.value()
    }

    #[inline]
    fn is_frozen(meta: ribbit::Packed<Self>) -> bool {
        meta.frozen()
    }

    #[inline]
    fn with_frozen(meta: ribbit::Packed<Self>, frozen: bool) -> ribbit::Packed<Self> {
        meta.with_frozen(frozen)
    }

    #[inline]
    fn with_value(meta: ribbit::Packed<Self>, value: bool) -> ribbit::Packed<Self> {
        meta.with_value(value)
    }

    #[inline]
    fn expand(
        old: ribbit::Packed<Self>,
        new: Self::Key,
    ) -> Result<(ribbit::Packed<Self>, u8, ribbit::Packed<Self>), ()> {
        if Self::key(old) == new {
            return Err(());
        }

        let len = old.len().min(new.len());

        let len_start = unsafe {
            u6::new_unchecked(
                old.value
                    .bitxor(new.value)
                    .bitor(1u64.rotate_right(1) >> len.value())
                    .leading_zeros() as u8
                    & !0b111u8,
            )
        };

        let len_middle = unsafe { u6::new_unchecked(len_start.value() + 8) };
        Ok((
            Self::key_from_u64_truncate(old.value, len_start).with_value(false),
            old.value.rotate_left(len_middle.value() as u32) as u8,
            Self::key_from_u64_truncate(old.value << len_middle.value(), old.len() - len_middle)
                .with_value(old.value()),
        ))
    }

    #[inline]
    fn compress(
        parent: ribbit::Packed<Self>,
        byte: u8,
        child: ribbit::Packed<Self>,
    ) -> Option<ribbit::Packed<Self>> {
        todo!()
    }
}
