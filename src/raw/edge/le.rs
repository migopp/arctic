#![expect(dead_code)]

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
    pub(crate) fn key_from_u64_truncate(_value: u64, _len: u6) -> ribbit::Packed<Self> {
        todo!()
    }

    #[inline]
    pub(crate) fn min_len(_len: u6, _bits: usize) -> u6 {
        todo!()
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
    fn expand(self, _new: Self::Key) -> Result<(Self, u8, Self), ()> {
        todo!()
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
