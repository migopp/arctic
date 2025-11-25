use core::arch::x86_64::__m128i;
use core::arch::x86_64::_mm_add_epi8;
use core::arch::x86_64::_mm_cmpeq_epi8;
use core::arch::x86_64::_mm_cmpgt_epi8;
use core::arch::x86_64::_mm_cvtsi128_si64x;
use core::arch::x86_64::_mm_extract_epi64;
use core::arch::x86_64::_mm_max_epu8;
use core::arch::x86_64::_mm_min_epu8;
use core::arch::x86_64::_mm_movemask_epi8;
use core::arch::x86_64::_mm_set1_epi8;
use core::arch::x86_64::_mm_subs_epi8;
use core::arch::x86_64::_mm_unpackhi_epi8;
use core::arch::x86_64::_mm_unpacklo_epi8;
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

/// Output has 8 bits set for each byte in `array` that is non-zero.
#[inline(always)]
pub(super) fn mask_nonzero(array: u128) -> u128 {
    const ZERO: __m128i = u128_to_avx(0);
    let array = u128_to_avx(array);
    avx_to_u128(unsafe { _mm_cmpgt_epi8(array, ZERO) })
}

// https://talkchess.com/viewtopic.php?t=78804
// https://stackoverflow.com/questions/72098296/how-to-create-a-left-packed-vector-of-indices-of-the-0s-in-one-simd-vector
// http://const.me/articles/simd/simd.pdf
#[inline(always)]
pub(super) fn compress(data: u128, mask: u128) -> [u128; 2] {
    let meta = 0x0F0E0D0C0B0A09080706050403020100u128;

    let (data_lo, data_hi) = split(data);
    let (meta_lo, meta_hi) = split(meta);
    let (mask_lo, mask_hi) = split(mask);
    let shift = mask_lo.count_ones();

    let data_lo = unsafe { _pext_u64(data_lo, mask_lo) };
    let data_hi = unsafe { _pext_u64(data_hi, mask_hi) };
    let data = u128_to_avx((data_lo as u128) | (data_hi as u128).wrapping_shl(shift));

    let meta_lo = unsafe { _pext_u64(meta_lo, mask_lo) };
    let meta_hi = unsafe { _pext_u64(meta_hi, mask_hi) };
    let meta = u128_to_avx((meta_lo as u128) | (meta_hi as u128).wrapping_shl(shift));

    [
        avx_to_u128(unsafe { _mm_unpacklo_epi8(data, meta) }),
        avx_to_u128(unsafe { _mm_unpackhi_epi8(data, meta) }),
    ]
}

// https://talkchess.com/viewtopic.php?t=78804
// https://stackoverflow.com/questions/72098296/how-to-create-a-left-packed-vector-of-indices-of-the-0s-in-one-simd-vector
// http://const.me/articles/simd/simd.pdf
#[inline(always)]
pub(super) fn compress_47(data: u128, offset: u8, mask: u128) -> [u128; 2] {
    let meta = unsafe {
        avx_to_u128(_mm_add_epi8(
            u128_to_avx(0x0F0E0D0C0B0A09080706050403020100u128),
            _mm_set1_epi8(offset as i8),
        ))
    };

    let data = u128_to_avx(data);
    let ones = unsafe { _mm_set1_epi8(1) };
    let data = avx_to_u128(unsafe { _mm_subs_epi8(data, ones) });

    let (data_lo, data_hi) = split(data);
    let (meta_lo, meta_hi) = split(meta);
    let (mask_lo, mask_hi) = split(mask);
    let shift = mask_lo.count_ones();

    let meta_lo = unsafe { _pext_u64(meta_lo, mask_lo) };
    let meta_hi = unsafe { _pext_u64(meta_hi, mask_hi) };
    let meta = u128_to_avx((meta_lo as u128) | (meta_hi as u128).wrapping_shl(shift));

    let data_lo = unsafe { _pext_u64(data_lo, mask_lo) };
    let data_hi = unsafe { _pext_u64(data_hi, mask_hi) };
    let data = u128_to_avx((data_lo as u128) | (data_hi as u128).wrapping_shl(shift));

    [
        avx_to_u128(unsafe { _mm_unpacklo_epi8(meta, data) }),
        avx_to_u128(unsafe { _mm_unpackhi_epi8(meta, data) }),
    ]
}

#[inline(always)]
fn split(value: u128) -> (u64, u64) {
    let value = u128_to_avx(value);
    let lo = unsafe { _mm_cvtsi128_si64x(value) } as u64;
    let hi = unsafe { _mm_extract_epi64::<1>(value) } as u64;
    (lo, hi)
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
