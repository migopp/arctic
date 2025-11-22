use ribbit::u120;
use ribbit::u4;

use crate::raw::edge;
use crate::raw::node;
use crate::raw::node::linear;
use crate::raw::node::Node3;
use crate::raw::node::Node60;
use crate::raw::Edge;

pub(crate) type Node15<C> = super::Linear<15, Header, C>;

const _: () = assert!(core::mem::size_of::<Node15<()>>() == 256);
const _: () = assert!(core::mem::align_of::<Node15<()>>() == 64);

#[derive(Copy, Clone, Debug, ribbit::Pack)]
#[ribbit(size = 128, packed(rename = "HeaderPacked"), debug)]
pub(crate) struct Header {
    keys: u120,
    len: u4,
    frozen: bool,
}

impl Header {
    const DEFAULT: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(u120::new(0), u4::new(0), false);
}

impl Default for HeaderPacked {
    fn default() -> Self {
        Header::DEFAULT
    }
}

impl linear::Header for ribbit::Packed<Header> {
    const KIND: node::Kind = node::Kind::Node15;
    const GROW: usize = 15;

    type Grow<M>
        = Node60<M>
    where
        M: ribbit::Pack<Packed: edge::Meta>;
    type Shrink<M>
        = Node3<M>
    where
        M: ribbit::Pack<Packed: edge::Meta>;

    type KeyBuffer = [u8; 15];
    type EdgeBuffer<M>
        = [ribbit::Packed<Edge<M>>; 15]
    where
        M: ribbit::Pack<Packed: edge::Meta>;

    fn buffer<M: ribbit::Pack<Packed: edge::Meta>>() -> (Self::KeyBuffer, Self::EdgeBuffer<M>) {
        (
            core::array::from_fn(|_| 0),
            core::array::from_fn(|_| Edge::DEFAULT),
        )
    }

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
        validate!(len <= 15);
        match self.get(key) {
            Some(index) => Ok(index),
            _ if len == 15 || self.is_frozen() => Err(None),
            _ => Err(Some(Self::new(
                u120::new(self.keys().value() | ((key as u128) << (len * 8))),
                u4::new(len + 1),
                false,
            ))),
        }
    }

    fn keys_unsorted(self) -> linear::KeyIter {
        let keys = self.value.to_le_bytes();
        let len = self.len().value();
        let indexes: [(u8, u8); 15] = core::array::from_fn(|index| (keys[index], index as u8));
        linear::KeyIter::new_15(linear::RawIter::new(indexes, len))
    }

    fn keys_sorted(self) -> linear::KeyIter {
        let keys = self.value.to_le_bytes();
        let len = self.len().value();
        let mut indexes: [(u8, u8); 15] = core::array::from_fn(|index| (keys[index], index as u8));
        indexes[..len as usize].sort_unstable();
        linear::KeyIter::new_15(linear::RawIter::new(indexes, len))
    }

    fn keys_range<L: crate::raw::node::Lower, H: crate::raw::node::Upper>(
        self,
        low: L,
        high: H,
    ) -> linear::KeyIter {
        // https://stackoverflow.com/a/28383095
        // https://talkchess.com/viewtopic.php?t=78804
        let (keys, len) = unsafe {
            use core::arch::x86_64::_mm_and_si128;
            use core::arch::x86_64::_mm_cmpeq_epi8;
            use core::arch::x86_64::_mm_max_epu8;
            use core::arch::x86_64::_mm_min_epu8;
            use core::arch::x86_64::_mm_set1_epi8;

            let keys = core::mem::transmute::<u128, core::arch::x86_64::__m128i>(self.value);
            let len = self.len().value() as usize;

            let mask_len = core::mem::transmute::<u128, core::arch::x86_64::__m128i>(
                (1u128 << (len << 3)) - 1,
            );

            let min = low.get();
            let max = high.get();

            let min = _mm_set1_epi8(min as i8);
            let max = _mm_set1_epi8(max as i8);
            let mask_range = _mm_cmpeq_epi8(_mm_min_epu8(_mm_max_epu8(min, keys), max), keys);

            let mask_valid = core::mem::transmute::<core::arch::x86_64::__m128i, u128>(
                _mm_and_si128(mask_len, mask_range),
            );
            let len = (mask_valid.count_ones() >> 3) as u8;

            (self.value & mask_valid | !mask_valid, len)
        };

        // TODO: SIMD sorting network?
        let keys = keys.to_le_bytes();
        let mut indexes: [(u8, u8); 15] = core::array::from_fn(|index| (keys[index], index as u8));
        indexes.sort_unstable();
        linear::KeyIter::new_15(linear::RawIter::new(indexes, len))
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
        assert_eq!(super::get_naive(array, key), expected);

        #[cfg(all(target_arch = "x86_64", target_feature = "sse2"))]
        assert_eq!(super::get_simd(array, key), expected);
    }
}
