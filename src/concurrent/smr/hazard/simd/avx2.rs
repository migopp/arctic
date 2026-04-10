use crate::concurrent::smr::hazard::Prefix;

pub fn check_hazard<P: ribbit::Pack<Packed: Prefix>>(
    snapshot: &[ribbit::Packed<P>],
    prefix: ribbit::Packed<P>,
) -> bool {
    snapshot.chunks(4).any(|hazard| {
        let hazards = [
            hazard.get(0).copied().unwrap_or(Prefix::HAZARD_NULL),
            hazard.get(1).copied().unwrap_or(Prefix::HAZARD_NULL),
            hazard.get(2).copied().unwrap_or(Prefix::HAZARD_NULL),
            hazard.get(3).copied().unwrap_or(Prefix::HAZARD_NULL),
        ];
        prefix.is_conflict_simd(hazards)
    })
}
