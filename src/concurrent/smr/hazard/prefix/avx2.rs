use crate::concurrent::smr::hazard::prefix::BePacked;
use crate::concurrent::smr::hazard::prefix::LePacked;

impl BePacked {
    pub(super) fn is_conflict_avx2(self, prefix: &[Self; 4]) -> bool {
        use core::arch::x86_64::*;
        validate!(self.node() ^ self.value());

        unsafe {
            // set hazard and broadcast prefix
            let h = _mm256_set_epi64x(
                prefix[3].value() as i64,
                prefix[2].value() as i64,
                prefix[1].value() as i64,
                prefix[0].value() as i64,
            );
            let p = _mm256_set1_epi64x(self.value() as i64);

            let zeros = _mm256_setzero_si256();
            let ones = _mm256_set1_epi64x(-1i64);

            // Case: `hazard` doesn't protect node or value
            // (h & p) & (0b11 << 56)
            let type_bits =
                _mm256_and_si256(_mm256_and_si256(h, p), _mm256_set1_epi64x(0b11 << 56));
            // != 0
            let type_match = _mm256_xor_si256(_mm256_cmpeq_epi64(type_bits, zeros), ones);

            // Case: `hazard` protects prefixes only, and `prefix` is higher up the tree
            // get overlap bit
            let h_no_overlap =
                _mm256_cmpeq_epi64(_mm256_and_si256(h, _mm256_set1_epi64x(0b100 << 56)), zeros);

            // fetch len
            let h_bits =
                _mm256_and_si256(_mm256_srli_epi64::<56>(h), _mm256_set1_epi64x(0b111_000));
            let p_bits =
                _mm256_and_si256(_mm256_srli_epi64::<56>(p), _mm256_set1_epi64x(0b111_000));

            // !h.overlap() && h.len > p.len
            let skip = _mm256_and_si256(h_no_overlap, _mm256_cmpgt_epi64(h_bits, p_bits));

            // Case: Overlapping prefix
            // h ^ p
            let xor = _mm256_xor_si256(h, p);

            let bits = _mm256_min_epu16(h_bits, p_bits);

            // Be::extract logic
            let one = _mm256_set1_epi64x(1);
            let prefix_mask = _mm256_sub_epi64(_mm256_sllv_epi64(one, bits), one);

            let overlap = _mm256_cmpeq_epi64(_mm256_and_si256(xor, prefix_mask), zeros);

            // combine: type_match & !skip & overlap
            let result = _mm256_and_si256(
                type_match,
                _mm256_and_si256(_mm256_andnot_si256(skip, ones), overlap),
            );
            _mm256_testz_si256(result, result) == 0
        }
    }
}

impl LePacked {
    pub(super) fn is_conflict_avx2(self, prefix: &[Self; 4]) -> bool {
        use core::arch::x86_64::*;
        validate!(self.node() ^ self.value());

        unsafe {
            // set hazard and broadcast prefix
            let h = _mm256_set_epi64x(
                prefix[3].value() as i64,
                prefix[2].value() as i64,
                prefix[1].value() as i64,
                prefix[0].value() as i64,
            );
            let p = _mm256_set1_epi64x(self.value() as i64);

            let zeros = _mm256_setzero_si256();
            let ones = _mm256_set1_epi64x(-1i64);

            // Case: `hazard` doesn't protect node or value
            // (h & p) & 0b11
            let type_bits = _mm256_and_si256(_mm256_and_si256(h, p), _mm256_set1_epi64x(0b11));

            // != 0
            let type_match = _mm256_xor_si256(_mm256_cmpeq_epi64(type_bits, zeros), ones);

            // Case: `hazard` protects prefixes only, and `prefix` is higher up the tree
            // get overlap bit
            let h_no_overlap =
                _mm256_cmpeq_epi64(_mm256_and_si256(h, _mm256_set1_epi64x(0b100)), zeros);

            // fetch len
            let h_bits = _mm256_and_si256(h, _mm256_set1_epi64x(0b111_000));
            let p_bits = _mm256_and_si256(p, _mm256_set1_epi64x(0b111_000));
            let skip = _mm256_and_si256(h_no_overlap, _mm256_cmpgt_epi64(h_bits, p_bits)); // !h.overlap() && h.len > p.len

            // Case: Overlapping prefix
            // h ^ p
            let xor = _mm256_xor_si256(h, p);

            let bits = _mm256_min_epu16(h_bits, p_bits);

            // Be::extract logic
            let shift = _mm256_sub_epi64(_mm256_set1_epi64x(64), bits);
            let prefix_mask = _mm256_sllv_epi64(ones, shift);

            // (xor & mask) == 0
            let overlap = _mm256_cmpeq_epi64(_mm256_and_si256(xor, prefix_mask), zeros);

            // combine, type_match & !skip & overlap
            let result = _mm256_and_si256(
                type_match,
                _mm256_and_si256(_mm256_andnot_si256(skip, ones), overlap),
            );
            _mm256_testz_si256(result, result) == 0
        }
    }
}
