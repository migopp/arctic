use core::arch::x86_64::__m128i;
use core::arch::x86_64::_mm_adds_epu8;
use core::arch::x86_64::_mm_and_si128;
use core::arch::x86_64::_mm_cmpeq_epi8;
use core::arch::x86_64::_mm_cmplt_epi8;
use core::arch::x86_64::_mm_cvtsi128_si64x;
use core::arch::x86_64::_mm_extract_epi64;
use core::arch::x86_64::_mm_max_epu8;
use core::arch::x86_64::_mm_min_epu8;
use core::arch::x86_64::_mm_movemask_epi8;
use core::arch::x86_64::_mm_mullo_epi16;
use core::arch::x86_64::_mm_set1_epi16;
use core::arch::x86_64::_mm_set1_epi8;
use core::arch::x86_64::_mm_set_epi64x;
use core::arch::x86_64::_mm_slli_epi16;
use core::arch::x86_64::_mm_srli_epi16;
use core::arch::x86_64::_mm_unpackhi_epi8;
use core::arch::x86_64::_mm_unpacklo_epi8;
use core::arch::x86_64::_pext_u64;

use crate::raw::node::iter::KeyIndex;

/// Output has 8 bits set for each byte in `array` that is equal to `byte`.
#[inline(always)]
pub(super) fn mask_eq(array: u128, byte: u8) -> u128 {
    let array = u128_to_avx(array);
    let byte = unsafe { _mm_set1_epi8(byte as i8) };
    avx_to_u128(unsafe { _mm_cmpeq_epi8(array, byte) })
}

/// Output has 8 bits set for each byte in `array` that is less than `byte` (signed).
#[inline(always)]
pub(super) fn mask_lt(array: u128, byte: i8) -> u128 {
    let array = u128_to_avx(array);
    let byte = unsafe { _mm_set1_epi8(byte) };
    avx_to_u128(unsafe { _mm_cmplt_epi8(array, byte) })
}

/// Output has 8 bits set for each byte in `array` that is within `min..=max` (unsigned).
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

/// Output has 8 bits set for each byte in `array` below `len`
#[inline(always)]
pub(super) fn mask_len(len: u8) -> u128 {
    avx_to_u128(unsafe { _mm_cmplt_epi8(u128_to_avx(U8_SEQ), _mm_set1_epi8(len as i8)) })
}

/// Convert byte mask to bit mask
#[inline(always)]
pub(super) fn mask_byte_to_bit(mask: u128) -> u16 {
    unsafe { _mm_movemask_epi8(u128_to_avx(mask)) as u16 }
}

// https://talkchess.com/viewtopic.php?t=78804
// https://stackoverflow.com/questions/72098296/how-to-create-a-left-packed-vector-of-indices-of-the-0s-in-one-simd-vector
// http://const.me/articles/simd/simd.pdf
#[inline(always)]
pub(super) fn compress(keys: u128, indices: u128, mask: u128) -> [KeyIndex; 16] {
    let (ks_lo, ks_hi) = split(keys);
    let (is_lo, is_hi) = split(indices);
    let (mask_lo, mask_hi) = split(mask);
    let shift = mask_lo.count_ones();

    let ks_lo = unsafe { _pext_u64(ks_lo, mask_lo) };
    let ks_hi = unsafe { _pext_u64(ks_hi, mask_hi) };
    let is_lo = unsafe { _pext_u64(is_lo, mask_lo) };
    let is_hi = unsafe { _pext_u64(is_hi, mask_hi) };

    validate!(shift <= 64);

    let ks_hi_hi = ks_hi.unbounded_shr(64 - shift);
    let is_hi_hi = is_hi.unbounded_shr(64 - shift);

    let ks_hi_lo = ks_hi.unbounded_shl(shift);
    let is_hi_lo = is_hi.unbounded_shl(shift);

    let ks = unsafe { _mm_set_epi64x(ks_hi_hi as i64, (ks_hi_lo | ks_lo) as i64) };
    let is = unsafe { _mm_set_epi64x(is_hi_hi as i64, (is_hi_lo | is_lo) as i64) };

    let out = interleave(avx_to_u128(ks), avx_to_u128(is));
    let out = core::array::from_fn(|i| out[i].to_le_bytes());
    unsafe { core::mem::transmute::<[[u8; 16]; 2], [KeyIndex; 16]>(out) }
}

pub(super) const U8_16: u128 = 0x1010_1010_1010_1010_1010_1010_1010_1010u128;
pub(super) const U8_SEQ: u128 = 0x0F0E_0D0C_0B0A_0908_0706_0504_0302_0100u128;

// https://stackoverflow.com/a/29155682
#[inline(always)]
pub(super) fn mul(a: u128, b: u8) -> u128 {
    let a = u128_to_avx(a);
    let b = unsafe { _mm_set1_epi8(b as i8) };

    let even = avx_to_u128(unsafe { _mm_and_si128(_mm_mullo_epi16(a, b), _mm_set1_epi16(0xFF)) });
    let odd = avx_to_u128(unsafe {
        _mm_slli_epi16::<8>(_mm_mullo_epi16(
            _mm_srli_epi16::<8>(a),
            _mm_srli_epi16::<8>(b),
        ))
    });

    even | odd
}

#[inline(always)]
pub(super) fn add(a: u128, b: u128) -> u128 {
    avx_to_u128(unsafe { _mm_adds_epu8(u128_to_avx(a), u128_to_avx(b)) })
}

#[inline(always)]
fn interleave(lo: u128, hi: u128) -> [u128; 2] {
    let lo = u128_to_avx(lo);
    let hi = u128_to_avx(hi);

    let out_lo = avx_to_u128(unsafe { _mm_unpacklo_epi8(lo, hi) });
    let out_hi = avx_to_u128(unsafe { _mm_unpackhi_epi8(lo, hi) });

    [out_lo, out_hi]
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
        assert_eq!(
            super::mask_byte_to_bit(super::mask_eq(array, key)).trailing_zeros() as u8,
            expected
        );
    }
}
