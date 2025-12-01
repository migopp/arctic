use core::arch::x86_64::__m128i;
use core::arch::x86_64::__m256i;
use core::arch::x86_64::_mm256_blend_epi16;
use core::arch::x86_64::_mm256_extracti128_si256;
use core::arch::x86_64::_mm256_max_epu16;
use core::arch::x86_64::_mm256_min_epu16;
use core::arch::x86_64::_mm256_permute2x128_si256;
use core::arch::x86_64::_mm256_set_m128i;
use core::arch::x86_64::_mm256_setr_epi8;
use core::arch::x86_64::_mm256_shuffle_epi8;
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
use core::arch::x86_64::_mm_storeu_si128;
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
    let shift_lo = mask_lo.count_ones();
    let shift_hi = mask_hi.count_ones();

    let ks_lo = unsafe { _pext_u64(ks_lo, mask_lo) };
    let ks_hi = unsafe { _pext_u64(ks_hi, mask_hi) };
    let is_lo = unsafe { _pext_u64(is_lo, mask_lo) };
    let is_hi = unsafe { _pext_u64(is_hi, mask_hi) };

    validate!(shift_lo <= 64);
    validate!(shift_hi < 64);

    let ks_hi_hi = ks_hi.unbounded_shr(64 - shift_lo);
    let is_hi_hi = is_hi.unbounded_shr(64 - shift_lo);
    let fill_hi = !((1u64 << shift_hi.saturating_sub(64 - shift_lo)) - 1);

    let ks_hi_lo = ks_hi.unbounded_shl(shift_lo);
    let is_hi_lo = is_hi.unbounded_shl(shift_lo);
    let fill_lo = !1u64.unbounded_shl(shift_hi + shift_lo).wrapping_sub(1);

    let ks = unsafe {
        _mm_set_epi64x(
            (fill_hi | ks_hi_hi) as i64,
            (fill_lo | ks_hi_lo | ks_lo) as i64,
        )
    };
    let is = unsafe {
        _mm_set_epi64x(
            (fill_hi | is_hi_hi) as i64,
            (fill_lo | is_hi_lo | is_lo) as i64,
        )
    };

    let mut out = unsafe { _mm256_set_m128i(_mm_unpackhi_epi8(ks, is), _mm_unpacklo_epi8(ks, is)) };

    unsafe {
        out = bitonic_step::<SHUFFLE_1, BLEND_1>(out);

        out = bitonic_step::<SHUFFLE_2, BLEND_2>(out);
        out = bitonic_step::<SHUFFLE_1, BLEND_1>(out);

        out = bitonic_step::<SHUFFLE_4, BLEND_4>(out);
        out = bitonic_step::<SHUFFLE_2, BLEND_2>(out);
        out = bitonic_step::<SHUFFLE_1, BLEND_1>(out);

        out = bitonic_step::<SHUFFLE_8, BLEND_8>(out);
        out = bitonic_step::<SHUFFLE_4, BLEND_4>(out);
        out = bitonic_step::<SHUFFLE_2, BLEND_2>(out);
        out = bitonic_step::<SHUFFLE_1, BLEND_1>(out);

        out = _mm256_shuffle_epi8(
            out,
            _mm256_setr_epi8(
                0x1, 0x0, 0x3, 0x2, 0x5, 0x4, 0x7, 0x6, 0x9, 0x8, 0xB, 0xA, 0xD, 0xC, 0xF, 0xE,
                0x1, 0x0, 0x3, 0x2, 0x5, 0x4, 0x7, 0x6, 0x9, 0x8, 0xB, 0xA, 0xD, 0xC, 0xF, 0xE,
            ),
        );

        core::mem::transmute::<__m256i, [KeyIndex; 16]>(out)
    }
}

const SHUFFLE_1: u64 = 0x6745_2301;
const BLEND_1: i32 = 0b1010_1010;

const SHUFFLE_2: u64 = 0x5476_1032;
const BLEND_2: i32 = 0b1100_1100;

const SHUFFLE_4: u64 = 0x3210_7654;
const BLEND_4: i32 = 0b1111_0000;

// Dummy values--shuffling across lanes requires different intrinsic
const SHUFFLE_8: u64 = 0xFFFF_FFFF;
const BLEND_8: i32 = 0b1111_1111;

/// https://en.wikipedia.org/wiki/Bitonic_sorter
/// https://github.com/Geolm/simd_bitonic
/// https://hal.inria.fr/hal-01512970v1/document
#[inline(always)]
fn bitonic_step<const SHUFFLE: u64, const BLEND: i32>(input: __m256i) -> __m256i {
    const fn extract(shuffle: u64, index: u8) -> i8 {
        // `% 16` to repeat across lanes, `/ 2` for u16 granularity, `/ 4` for bit width
        let shift = (index % 16 / 2) * 4;
        let select = (shuffle >> shift) & 0b1111;
        // Mix bit from top/bottom u16 back in
        ((select << 1) | (index as u64 & 1)) as i8
    }

    let shuffle = unsafe {
        _mm256_setr_epi8(
            const { extract(SHUFFLE, 0) },
            const { extract(SHUFFLE, 1) },
            const { extract(SHUFFLE, 2) },
            const { extract(SHUFFLE, 3) },
            const { extract(SHUFFLE, 4) },
            const { extract(SHUFFLE, 5) },
            const { extract(SHUFFLE, 6) },
            const { extract(SHUFFLE, 7) },
            const { extract(SHUFFLE, 8) },
            const { extract(SHUFFLE, 9) },
            const { extract(SHUFFLE, 10) },
            const { extract(SHUFFLE, 11) },
            const { extract(SHUFFLE, 12) },
            const { extract(SHUFFLE, 13) },
            const { extract(SHUFFLE, 14) },
            const { extract(SHUFFLE, 15) },
            const { extract(SHUFFLE, 16) },
            const { extract(SHUFFLE, 17) },
            const { extract(SHUFFLE, 18) },
            const { extract(SHUFFLE, 19) },
            const { extract(SHUFFLE, 20) },
            const { extract(SHUFFLE, 21) },
            const { extract(SHUFFLE, 22) },
            const { extract(SHUFFLE, 23) },
            const { extract(SHUFFLE, 24) },
            const { extract(SHUFFLE, 25) },
            const { extract(SHUFFLE, 26) },
            const { extract(SHUFFLE, 27) },
            const { extract(SHUFFLE, 28) },
            const { extract(SHUFFLE, 29) },
            const { extract(SHUFFLE, 30) },
            const { extract(SHUFFLE, 31) },
        )
    };

    let swap = if SHUFFLE == SHUFFLE_8 {
        unsafe { _mm256_permute2x128_si256::<0b0000_0001>(input, input) }
    } else {
        unsafe { _mm256_shuffle_epi8(input, shuffle) }
    };

    let min = unsafe { _mm256_min_epu16(input, swap) };
    let max = unsafe { _mm256_max_epu16(input, swap) };

    if BLEND == BLEND_8 {
        unsafe {
            _mm256_set_m128i(
                _mm256_extracti128_si256::<1>(max),
                _mm256_extracti128_si256::<0>(min),
            )
        }
    } else {
        unsafe { _mm256_blend_epi16::<BLEND>(min, max) }
    }
}

#[inline(always)]
pub(super) unsafe fn compress_into(keys: u128, indices: u128, mask: u128, buffer: *mut KeyIndex) {
    let (ks_lo, ks_hi) = split(keys);
    let (is_lo, is_hi) = split(indices);
    let (mask_lo, mask_hi) = split(mask);
    let shift = mask_lo.count_ones();

    let ks_lo = unsafe { _pext_u64(ks_lo, mask_lo) };
    let ks_hi = unsafe { _pext_u64(ks_hi, mask_hi) };
    let is_lo = unsafe { _pext_u64(is_lo, mask_lo) };
    let is_hi = unsafe { _pext_u64(is_hi, mask_hi) };

    let ks = unsafe { _mm_set_epi64x(ks_hi as i64, ks_lo as i64) };
    let is = unsafe { _mm_set_epi64x(is_hi as i64, is_lo as i64) };
    let [lo, hi] = interleave(avx_to_u128(ks), avx_to_u128(is));

    // FIXME: assumes little-endian
    unsafe {
        _mm_storeu_si128(buffer.cast(), u128_to_avx(lo));
        _mm_storeu_si128(
            buffer.byte_add((shift >> 2) as usize).cast(),
            u128_to_avx(hi),
        );
    }
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
