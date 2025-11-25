use core::arch::x86_64::__m128i;
use core::arch::x86_64::_mm_cmpeq_epi8;
use core::arch::x86_64::_mm_cvtsi128_si64x;
use core::arch::x86_64::_mm_cvtsi64_si128;
use core::arch::x86_64::_mm_extract_epi64;
use core::arch::x86_64::_mm_max_epu8;
use core::arch::x86_64::_mm_min_epu8;
use core::arch::x86_64::_mm_movemask_epi8;
use core::arch::x86_64::_mm_set1_epi8;
use core::arch::x86_64::_pext_u64;

/// Output has 1 bit set for each byte in `array` that is equal to `byte`.
#[inline(always)]
pub(super) fn mask_eq(array: u128, byte: u8) -> u16 {
    let array = u128_to_avx(array);
    let byte = unsafe { _mm_set1_epi8(byte as i8) };
    let mask = unsafe { _mm_movemask_epi8(_mm_cmpeq_epi8(array, byte)) };
    mask as u16
}

/// Output has 8 bits set for each byte in `array` that is within `min..=max`.
#[inline(always)]
pub(super) fn mask_range(array: u128, min: u8, max: u8) -> u128 {
    let array = u128_to_avx(array);

    let min = unsafe { _mm_set1_epi8(min as i8) };
    let max = unsafe { _mm_set1_epi8(max as i8) };

    // https://stackoverflow.com/a/28383095
    let clamp_min = unsafe { _mm_max_epu8(array, min) };
    let clamp = unsafe { _mm_min_epu8(clamp_min, max) };

    avx_to_u128(unsafe { _mm_cmpeq_epi8(array, clamp) })
}

// https://talkchess.com/viewtopic.php?t=78804
// https://stackoverflow.com/questions/72098296/how-to-create-a-left-packed-vector-of-indices-of-the-0s-in-one-simd-vector
#[inline(always)]
pub(super) fn compress(array: u128, mask: u128) -> u128 {
    let array = u128_to_avx(array);
    let mask = u128_to_avx(mask);

    let array_low = unsafe { _mm_cvtsi128_si64x(array) } as u64;
    let array_high = unsafe { _mm_extract_epi64::<1>(array) } as u64;

    let mask_low = unsafe { _mm_cvtsi128_si64x(mask) } as u64;
    let mask_high = unsafe { _mm_extract_epi64::<1>(mask) } as u64;

    let low = unsafe { _pext_u64(array_low, mask_low) };
    let high = unsafe { _pext_u64(array_high, mask_high) };
    (low as u128) | (high as u128).unbounded_shl(mask_low.count_ones())
}

#[inline(always)]
const fn avx_to_u128(value: __m128i) -> u128 {
    unsafe { core::mem::transmute::<__m128i, u128>(value) }
}

#[inline(always)]
const fn u128_to_avx(value: u128) -> __m128i {
    unsafe { core::mem::transmute::<u128, __m128i>(value) }
}

#[cfg(test)]
mod tests {
    #[test]
    fn zero() {
        test_eq(0x00_00_00_00, 0, 0)
    }

    #[test]
    fn zero_high() {
        test_eq(0x00_00_12_34, 0, 2)
    }

    #[test]
    fn nonzero_middle() {
        test_eq(0x00_11_12_13, 0x12, 1)
    }

    #[test]
    fn duplicate() {
        test_eq(0x00_11_11_12, 0x11, 1)
    }

    #[test]
    fn lsb() {
        test_eq(u128::MAX, 0xFF, 0)
    }

    #[test]
    fn msb() {
        test_eq(0x0F << 120, 0x0F, 15)
    }

    fn test_eq(array: u128, key: u8, expected: u8) {
        assert_eq!(super::mask_eq(array, key).trailing_zeros() as u8, expected);
    }
}
