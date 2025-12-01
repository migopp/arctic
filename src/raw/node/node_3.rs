use ribbit::u4;
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
    len: u4,
}

impl Header {
    const DEFAULT: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(u48::new(0), false, u4::new(0));
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
        let index = get(self.value, key);
        (index < self.len().value()).then_some(index)
    }

    #[inline]
    fn get_or_insert(self, key: u8) -> Result<u8, Option<Self>> {
        let len = self.len().value();
        match self.get(key) {
            Some(index) if index < len => Ok(index),
            _ if len >= 3 || self.is_frozen() => Err(None),
            _ => Err(Some(Self::new(
                u48::new(self.keys().value() | ((key as u64) << (len << 4))),
                false,
                u4::new(len + 1),
            ))),
        }
    }

    fn keys<L: crate::raw::node::Lower, U: crate::raw::node::Upper>(
        self,
        lower: L,
        upper: U,
    ) -> node::KeyIter {
        let len = self.len().value();

        let mask_len = (1u64 << (len << 4)) - 1;
        let mask_valid = if lower.get() == 0 && upper.get() == 255 {
            mask_len
        } else {
            mask_len & node::simd::mask_range_4(self.value, lower.get(), upper.get())
        };
        let (len, out) = node::simd::compress_4(self.value, mask_valid);

        let entries = core::array::from_fn(|i| out[i]);
        node::KeyIter::new_3(linear::KeyIter3::new_3(entries, len))
    }
}

#[inline]
fn get(array: u64, key: u8) -> u8 {
    if cfg!(feature = "opt-node3-get") {
        get_swar(array, key)
    } else {
        get_naive(array, key)
    }
}

/// https://richardstartin.github.io/posts/finding-bytes
/// https://orlp.net/blog/extracting-depositing-bits/
/// https://lemire.me/blog/2022/01/21/swar-explained-parsing-eight-digits/
/// https://lamport.azurewebsites.net/pubs/multiple-byte.pdf
#[inline]
fn get_swar(array: u64, key: u8) -> u8 {
    const LOWER: u64 = 0x0000_00FF_00FF_00FF;
    const OVERFLOW: u64 = 0x0000_0100_0100_0100;

    // Convert key bytes to zero
    let key_to_zero = array ^ broadcast(key);

    // Set overflow bit for byte if byte is non-zero
    let equal_zero = key_to_zero + LOWER;

    // Extract overflow bits
    unsafe { core::arch::x86_64::_pext_u64(equal_zero, OVERFLOW) }.trailing_ones() as u8
}

#[inline]
const fn broadcast(byte: u8) -> u64 {
    let byte = byte as u64;

    // LLVM is smart enough to turn this into an `imul`
    byte | (byte << 16) | (byte << 32)
}

#[inline]
fn get_naive(array: u64, key: u8) -> u8 {
    array
        .to_le_bytes()
        .into_iter()
        .step_by(2)
        .position(|byte| byte == key)
        .map(|index| index as u8)
        .unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
    use crate::raw::node::node_3;

    #[test]
    fn zero() {
        test_get(0x0000_0000_0000_0000, 0, 0)
    }

    #[test]
    fn zero_high() {
        test_get(0x0000_0000_0012_0034, 0, 2)
    }

    #[test]
    fn nonzero_middle() {
        test_get(0x00_0011_0012_0013, 0x12, 1)
    }

    #[test]
    fn duplicate() {
        test_get(0x0000_0011_0011_0012, 0x11, 1)
    }

    fn test_get(array: u64, key: u8, expected: u8) {
        assert_eq!(node_3::get_naive(array, key), expected);
        assert_eq!(node_3::get_swar(array, key), expected);
    }
}
