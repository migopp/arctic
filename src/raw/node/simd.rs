use core::arch::x86_64::__m128i;
use core::arch::x86_64::_mm_cmpeq_epi8;
use core::arch::x86_64::_mm_max_epu8;
use core::arch::x86_64::_mm_min_epu8;
use core::arch::x86_64::_mm_set1_epi8;

// https://stackoverflow.com/a/28383095
#[inline(always)]
pub(super) fn byte_in_range(array: u128, min: u8, max: u8) -> u128 {
    let array = u128_to_avx(array);

    let min = unsafe { _mm_set1_epi8(min as i8) };
    let max = unsafe { _mm_set1_epi8(max as i8) };

    let clamp_min = unsafe { _mm_max_epu8(array, min) };
    let clamp = unsafe { _mm_min_epu8(clamp_min, max) };

    avx_to_u128(unsafe { _mm_cmpeq_epi8(array, clamp) })
}

#[inline(always)]
const fn avx_to_u128(value: __m128i) -> u128 {
    unsafe { core::mem::transmute::<__m128i, u128>(value) }
}

#[inline(always)]
const fn u128_to_avx(value: u128) -> __m128i {
    unsafe { core::mem::transmute::<u128, __m128i>(value) }
}
