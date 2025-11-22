use ribbit::u120;
use ribbit::u4;

use crate::raw::edge;
use crate::raw::node;
use crate::raw::node::linear;
use crate::raw::node::Node3;
use crate::raw::node::Node60;

pub(crate) type Node15<C> = super::Linear<15, Header, C>;

const _: () = assert!(core::mem::size_of::<Node15<()>>() == 256);
const _: () = assert!(core::mem::align_of::<Node15<()>>() == 64);

#[derive(Copy, Clone, Debug, ribbit::Pack)]
#[ribbit(size = 128, packed(rename = "HeaderPacked"), debug)]
pub(crate) struct Header {
    keys: u120,
    frozen: bool,
    len: u4,
}

impl Header {
    const DEFAULT: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(u120::new(0), false, u4::new(0));
}

impl Default for HeaderPacked {
    fn default() -> Self {
        Header::DEFAULT
    }
}

impl linear::Header for ribbit::Packed<Header> {
    const KIND: node::Kind = node::Kind::Node15;
    const LEN: usize = 15;

    type Grow<M>
        = Node60<M>
    where
        M: ribbit::Pack<Packed: edge::Meta>;
    type Shrink<M>
        = Node3<M>
    where
        M: ribbit::Pack<Packed: edge::Meta>;

    fn freeze(self) -> Self {
        self.with_frozen(true)
    }

    fn is_frozen(self) -> bool {
        self.frozen()
    }

    fn len(self) -> usize {
        self.len().value() as usize
    }

    fn get(self, key: u8) -> Option<u8> {
        let index = get(self.value, key);
        (index < self.len().value()).then_some(index)
    }

    fn get_or_insert(self, key: u8) -> Result<u8, Option<Self>> {
        let len = self.len().value();
        validate!(len <= Self::LEN as u8);
        match self.get(key) {
            Some(index) => Ok(index),
            _ if len == Self::LEN as u8 || self.is_frozen() => Err(None),
            _ => Err(Some(Self::new(
                u120::new(self.keys().value() | ((key as u128) << (len * 8))),
                false,
                u4::new(len + 1),
            ))),
        }
    }

    fn keys<L: crate::raw::node::Lower, U: crate::raw::node::Upper>(
        self,
        lower: L,
        upper: U,
    ) -> node::KeyIter {
        if L::UNBOUND && U::UNBOUND {
            let keys = self.value.to_le_bytes();
            let len = self.len().value();
            let mut entries: [(u8, u8); Self::LEN] =
                core::array::from_fn(|index| (keys[index], index as u8));
            entries[len as usize..].fill((0xFF, 0xFF));
            return node::KeyIter::from_node_15(linear::KeyIter::new(entries, len));
        }

        let len = self.len().value() as usize;
        let mask_len = (1u128 << (len << 3)) - 1;

        let mask_range = node::simd::byte_in_range(self.value, lower.get(), upper.get());
        let mask_valid = mask_len & mask_range;
        let len = (mask_valid.count_ones() >> 3) as u8;

        let keys = self.value & mask_valid | !mask_valid;

        // TODO: SIMD sorting network?
        let keys = keys.to_le_bytes();
        let entries: [(u8, u8); Self::LEN] =
            core::array::from_fn(|index| (keys[index], index as u8));
        node::KeyIter::from_node_15(linear::KeyIter::new(entries, len))
    }
}

#[inline]
fn get(array: u128, key: u8) -> u8 {
    if cfg!(feature = "opt-node15-get") {
        get_simd(array, key)
    } else {
        get_naive(array, key)
    }
}

#[inline]
#[cfg(all(target_arch = "x86_64", target_feature = "sse2"))]
fn get_simd(array: u128, key: u8) -> u8 {
    use core::arch::x86_64::_mm_cmpeq_epi8;
    use core::arch::x86_64::_mm_movemask_epi8;
    use core::arch::x86_64::_mm_set1_epi8;
    use std::arch::x86_64::__m128i;

    unsafe {
        _mm_movemask_epi8(_mm_cmpeq_epi8(
            core::mem::transmute::<u128, __m128i>(array),
            _mm_set1_epi8(key as i8),
        ))
        .trailing_zeros() as u8
    }
}

#[inline]
fn get_naive(array: u128, key: u8) -> u8 {
    array
        .to_le_bytes()
        .into_iter()
        .position(|byte| byte == key)
        .map(|index| index as u8)
        .unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
    use crate::raw::node::node_15;

    #[test]
    fn zero() {
        test_get(0x00_00_00_00, 0, 0)
    }

    #[test]
    fn zero_high() {
        test_get(0x00_00_12_34, 0, 2)
    }

    #[test]
    fn nonzero_middle() {
        test_get(0x00_11_12_13, 0x12, 1)
    }

    #[test]
    fn duplicate() {
        test_get(0x00_11_11_12, 0x11, 1)
    }

    #[test]
    fn lsb() {
        test_get(u128::MAX, 0xFF, 0)
    }

    #[test]
    fn msb() {
        test_get(0x0F << 120, 0x0F, 15)
    }

    fn test_get(array: u128, key: u8, expected: u8) {
        assert_eq!(node_15::get_naive(array, key), expected);

        #[cfg(all(target_arch = "x86_64", target_feature = "sse2"))]
        assert_eq!(node_15::get_simd(array, key), expected);
    }
}
