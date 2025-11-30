use crate::concurrent::hazard;
use crate::raw;
use crate::raw::key::dynamic;
use crate::raw::key::integer;
use crate::raw::key::Read as _;

pub trait Key: raw::Key {
    type Prefix: ribbit::Pack<Packed: hazard::Prefix>;

    fn hazard(reader: Self::Read<'_>) -> ribbit::Packed<Self::Prefix>;
}

impl Key for Vec<u8> {
    type Prefix = hazard::prefix::Be;

    #[inline]
    fn hazard(reader: Self::Read<'_>) -> ribbit::Packed<Self::Prefix> {
        hazard_dynamic(reader)
    }
}

impl Key for String {
    type Prefix = hazard::prefix::Be;

    #[inline]
    fn hazard(reader: Self::Read<'_>) -> ribbit::Packed<Self::Prefix> {
        hazard_dynamic(reader)
    }
}

macro_rules! impl_integer {
    ($($integer:ty),* $(,)?) => {
        $(
            impl Key for $integer {
                type Prefix = hazard::prefix::Be;

                #[inline]
                fn hazard(reader: Self::Read<'_>) -> ribbit::Packed<Self::Prefix> {
                    hazard_integer(reader)
                }
            }
        )*
    }
}

impl_integer!(u16, u32, u64, u128);

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
    let reader = reader.as_ref();
    let mut buffer = [0u8; 16];
    let len = reader.len().min(15);
    buffer[..len].copy_from_slice(&reader[..len]);
    crate::concurrent::hazard::prefix::Be::new_hazard(u128::from_be_bytes(buffer), len << 3)
}
