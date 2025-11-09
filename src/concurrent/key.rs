use crate::concurrent::hazard;
use crate::raw;
use crate::raw::key::dynamic;
use crate::raw::key::integer;
use crate::raw::key::Read as _;

pub trait Key: raw::Key {
    fn hazard(reader: Self::Read<'_>) -> ribbit::Packed<hazard::prefix::Be>;
}

impl Key for Vec<u8> {
    fn hazard(reader: Self::Read<'_>) -> ribbit::Packed<hazard::prefix::Be> {
        hazard_dynamic(reader)
    }
}

impl Key for String {
    fn hazard(reader: Self::Read<'_>) -> ribbit::Packed<hazard::prefix::Be> {
        hazard_dynamic(reader)
    }
}

macro_rules! impl_integer {
    ($($integer:ty),* $(,)?) => {
        $(
            impl Key for $integer {
                fn hazard(reader: Self::Read<'_>) -> ribbit::Packed<hazard::prefix::Be> {
                    hazard_integer(reader)
                }
            }
        )*
    }
}

impl_integer!(u16, u32, u64);

#[inline]
fn hazard_integer<U: integer::Uint>(
    reader: integer::Reader<U>,
) -> ribbit::Packed<crate::concurrent::hazard::prefix::Be> {
    crate::concurrent::hazard::prefix::Be::new_hazard(
        reader.buffer.most_significant_u128(),
        if U::BYTES < 16 {
            reader.bits()
        } else {
            reader.bits().min(120)
        },
    )
}

#[inline]
fn hazard_dynamic(
    reader: dynamic::Reader<'_>,
) -> ribbit::Packed<crate::concurrent::hazard::prefix::Be> {
    match reader {
        dynamic::Reader::Large(large) => {
            let mut buffer = [0u8; 16];
            let len = large.len().min(15);
            buffer[..len].copy_from_slice(&large[..len]);
            crate::concurrent::hazard::prefix::Be::new_hazard(u128::from_be_bytes(buffer), len << 3)
        }
        dynamic::Reader::Small(small) => hazard_integer(small),
    }
}
