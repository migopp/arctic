use crate::concurrent::smr::hazard::Prefix;

pub fn check_hazard<P: ribbit::Pack<Packed: Prefix>>(
    snapshot: &[ribbit::Packed<P>],
    prefix: ribbit::Packed<P>,
) -> bool {
    let (chunks, leftover) = snapshot.as_chunks::<4>();

    for hazard in chunks {
        if prefix.is_conflict_simd(*hazard) {
            return true;
        }
    }

    let hazards_leftover = [
        leftover.get(0).copied().unwrap_or(Prefix::HAZARD_NULL),
        leftover.get(1).copied().unwrap_or(Prefix::HAZARD_NULL),
        leftover.get(2).copied().unwrap_or(Prefix::HAZARD_NULL),
        leftover.get(3).copied().unwrap_or(Prefix::HAZARD_NULL),
    ];
    prefix.is_conflict_simd(hazards_leftover)
}
