use ribbit::u2;
use ribbit::u48;

use crate::raw::edge;
use crate::raw::node;
use crate::raw::node::linear;

use super::Node15;

pub(crate) type Node3<C> = super::Linear<3, Header, C>;

const _: () = assert!(core::mem::size_of::<Node3<()>>() == 64);
const _: () = assert!(core::mem::align_of::<Node3<()>>() == 64);

#[derive(Copy, Clone, Debug, ribbit::Pack)]
#[ribbit(size = 64, packed(rename = "HeaderPacked"), debug)]
pub(crate) struct Header {
    keys: u48,
    #[ribbit(offset = 48)]
    frozen: bool,
    #[ribbit(offset = 56)]
    len: u2,
}

impl Header {
    const DEFAULT: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(u48::new(0), false, u2::new(0));
}

impl Default for HeaderPacked {
    fn default() -> Self {
        Header::DEFAULT
    }
}

impl linear::Header for ribbit::Packed<Header> {
    const KIND: node::Kind = node::Kind::Node3;
    const LEN: usize = 3;

    type Grow<M>
        = Node15<M>
    where
        M: ribbit::Pack<Packed: edge::Meta>;
    type Shrink<M>
        = Node3<M>
    where
        M: ribbit::Pack<Packed: edge::Meta>;

    #[inline]
    fn freeze(self) -> Self {
        self.with_frozen(true)
    }

    #[inline]
    fn is_frozen(self) -> bool {
        self.frozen()
    }

    #[inline]
    fn len(self) -> u8 {
        self.len().value()
    }

    #[inline]
    fn get(self, key: u8) -> Option<u8> {
        let index = get_3(self.value, key);
        (index < self.len().value()).then_some(index)
    }

    #[inline]
    fn get_or_insert(self, key: u8) -> Result<u8, Option<Self>> {
        let index = get_3(self.value, key);
        let len = self.len().value();

        if index < len {
            return Ok(index);
        }

        if len >= Self::LEN as u8 || self.is_frozen() {
            return Err(None);
        }

        // Insert key byte and increment length
        let key = (key as u64) << (len << 4);
        let value = (self.value | key) + (1u64 << 56);

        // SAFETY: `len < Self::LEN`
        Err(Some(unsafe { Self::new_unchecked(value) }))
    }

    fn keys<L: crate::raw::node::Lower, U: crate::raw::node::Upper>(
        self,
        lower: L,
        upper: U,
    ) -> node::KeyIter {
        let len = self.len();
        let (len, out) = node::simd::compress_4(self.value, len, lower, upper);
        let entries = core::array::from_fn(|i| out[i]);
        node::KeyIter::new_3(linear::KeyIter3::new_3(entries, len))
    }
}

#[inline]
fn get_3(array: u64, key: u8) -> u8 {
    if cfg!(feature = "opt-no-node3-get") {
        get_3_fallback(array, key)
    } else {
        get_3_swar(array, key)
    }
}

/// https://richardstartin.github.io/posts/finding-bytes
/// https://orlp.net/blog/extracting-depositing-bits/
/// https://lemire.me/blog/2022/01/21/swar-explained-parsing-eight-digits/
/// https://lamport.azurewebsites.net/pubs/multiple-byte.pdf
#[inline]
fn get_3_swar(array: u64, key: u8) -> u8 {
    const LOWER: u64 = 0x0000_00FF_00FF_00FF;
    const OVERFLOW: u64 = 0x0000_0100_0100_0100;

    let key = key as u64;
    // LLVM is smart enough to turn this into an `imul`
    let key = key | (key << 16) | (key << 32);

    // Convert key bytes to zero
    let key_to_zero = array ^ key;

    // Set overflow bit for byte if byte is non-zero
    let equal_zero = key_to_zero + LOWER;

    // Extract overflow bits
    unsafe { core::arch::x86_64::_pext_u64(equal_zero, OVERFLOW) }.trailing_ones() as u8
}

#[inline]
fn get_3_fallback(array: u64, key: u8) -> u8 {
    array
        .to_le_bytes()
        .into_iter()
        .step_by(2)
        .position(|byte| byte == key)
        .map(|index| index as u8)
        .unwrap_or(3)
}

#[cfg(test)]
mod tests {
    use core::hash::Hasher as _;

    use crate::raw::node::node_3;

    #[test]
    fn get_3() {
        const COUNT: usize = 100_000;

        let mut hasher = rapidhash::fast::RapidHasher::default_const();

        for i in 0..COUNT {
            hasher.write_usize(i);
            let hash = hasher.finish();
            let array = hash & 0x00FF_00FF_00FF;
            let key = (hash >> 8) as u8;

            let swar = node_3::get_3_swar(array, key);
            let fallback = node_3::get_3_fallback(array, key);

            assert_eq!(
                swar, fallback,
                "SWAR {swar} does not match fallback {fallback} for array {array:x?} and key {key:x?}",
            );
        }
    }
}
