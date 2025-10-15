use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;
use ribbit::u120;
use ribbit::u4;

use crate::iter::Or;
use crate::node;
use crate::node::linear;
use crate::node::Node256;
use crate::node::Node3;

pub(crate) type Node15 = super::Linear<15, Atomic128<Header>>;

const _: () = assert!(core::mem::size_of::<Node15>() == 256);
const _: () = assert!(core::mem::align_of::<Node15>() == 64);

#[derive(Copy, Clone, Debug, Default, ribbit::Pack)]
#[ribbit(size = 128)]
pub(crate) struct Header {
    keys: u120,
    len: u4,
    frozen: bool,
}

impl linear::Header for Atomic128<Header> {
    fn freeze(&self) -> usize {
        let mut old = self.load_packed(Ordering::Relaxed);

        while !old.frozen() {
            match self.compare_exchange_packed(
                old,
                old.with_frozen(true),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(old) => return old.len().value() as usize,
                Err(conflict) => old = conflict,
            }
        }

        old.len().value() as usize
    }

    #[cold]
    fn get(&self, key: u8) -> Option<u8> {
        let header = self.load_packed(Ordering::Relaxed);
        let index = get(header.value, key);
        (index < header.len().value()).then_some(index)
    }

    #[cold]
    fn get_or_reserve(&self, key: u8) -> Option<u8> {
        let mut old = self.load_packed(Ordering::Acquire);

        loop {
            let index = get(old.value, key);
            let len = old.len().value();

            if index < len {
                return Some(index);
            } else if len >= 15 || old.frozen() {
                return None;
            }

            match self.compare_exchange_packed(
                old,
                ribbit::Packed::<Header>::new(
                    u120::new(old.keys().value() | ((key as u128) << (len * 8))),
                    u4::new(len + 1),
                    false,
                ),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(len),
                Err(conflict) => old = conflict,
            }
        }
    }

    #[cold]
    fn keys_range(&self, min: u8, max: u8) -> linear::SortedKeyIter {
        let header = self.load_packed(Ordering::Relaxed);

        // https://stackoverflow.com/a/28383095
        // https://talkchess.com/viewtopic.php?t=78804
        let (keys, len) = unsafe {
            use core::arch::x86_64::_mm_and_si128;
            use core::arch::x86_64::_mm_cmpeq_epi8;
            use core::arch::x86_64::_mm_cvtsi128_si64x;
            use core::arch::x86_64::_mm_extract_epi64;
            use core::arch::x86_64::_mm_max_epu8;
            use core::arch::x86_64::_mm_min_epu8;
            use core::arch::x86_64::_mm_set1_epi8;
            use core::arch::x86_64::_pext_u64;

            let keys = core::mem::transmute::<u128, core::arch::x86_64::__m128i>(header.value);
            let len = header.len().value() as usize;

            let within_len = core::mem::transmute::<u128, core::arch::x86_64::__m128i>(
                (1u128 << (len << 3)) - 1,
            );

            let min = _mm_set1_epi8(min as i8);
            let max = _mm_set1_epi8(max as i8);
            let within_range = _mm_cmpeq_epi8(_mm_min_epu8(_mm_max_epu8(min, keys), max), keys);

            let valid = _mm_and_si128(within_len, within_range);
            let len = (core::mem::transmute::<core::arch::x86_64::__m128i, u128>(valid)
                .count_ones()
                >> 3) as u8;

            let valid_low = _mm_cvtsi128_si64x(valid) as u64;
            let keys_low = _mm_cvtsi128_si64x(keys) as u64;
            let keys_low = _pext_u64(keys_low, valid_low);

            let valid_high = _mm_extract_epi64::<1>(valid) as u64;
            let keys_high = _mm_extract_epi64::<1>(keys) as u64;
            let keys_high = _pext_u64(keys_high, valid_high);

            let keys = ((keys_high as u128) << valid_low.count_ones()) | (keys_low as u128);
            (keys, len)
        };

        // TODO: SIMD sorting network?
        let keys = keys.to_le_bytes();
        let mut indexes: [(u8, u8); 15] = core::array::from_fn(|index| (keys[index], index as u8));
        indexes[..len as usize].sort_unstable();
        linear::SortedKeyIter::new_15(linear::RawKeyIter::new(indexes, len))
    }

    #[cold]
    fn keys_sorted(&self) -> linear::SortedKeyIter {
        let header = self.load_packed(Ordering::Relaxed);
        let keys = header.value.to_le_bytes();
        let len = header.len().value();
        let mut indexes: [(u8, u8); 15] = core::array::from_fn(|index| (keys[index], index as u8));
        indexes[..len as usize].sort_unstable();
        linear::SortedKeyIter::new_15(linear::RawKeyIter::new(indexes, len))
    }

    #[cold]
    fn keys_unsorted(&self) -> linear::UnsortedKeyIter {
        let header = self.load_packed(Ordering::Relaxed);
        Or::R(
            header
                .value
                .to_le_bytes()
                .into_iter()
                .take(header.len().value() as usize),
        )
    }
}

impl node::Info for Node15 {
    const KIND: node::Kind = node::Kind::Node15;
    const GROW: usize = 15;
    const REF: for<'a> fn(&'a Self) -> node::Ref<'a> = |node| node::Ref::Node15(node);

    type Grow = Node256;
    type Shrink = Node3;
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
