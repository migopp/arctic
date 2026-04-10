//#[cfg(target_feature = "avx2")]
mod avx2;

use crate::concurrent::smr::hazard::Prefix;

macro_rules! dispatch {
    ($avx2:expr, $fallback:expr $(,)?) => {{
        #[cfg(target_feature = "avx2")]
        {
            return $avx2;
        }

        #[allow(unreachable_code)]
        $fallback
    }};
}

pub fn check_hazard<P: ribbit::Pack<Packed: Prefix>, V>(
    snapshot: &[ribbit::Packed<P>],
    prefix: ribbit::Packed<P>,
) -> bool {
    dispatch!(
        avx2::check_hazard::<P, V>(snapshot, prefix),
        check_hazard_fallback::<P, V>(snapshot, prefix)
    )
}

fn check_hazard_fallback<P: ribbit::Pack<Packed: Prefix>, V>(
    snapshot: &[ribbit::Packed<P>],
    prefix: ribbit::Packed<P>,
) -> bool {
    snapshot.iter().any(|hazard| prefix.is_conflict(*hazard))
}
