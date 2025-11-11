use ribbit::u24;
use ribbit::u4;

use crate::raw::edge;
use crate::raw::node;
use crate::raw::node::linear;

use super::Node15;

pub(crate) type Node3<C> = super::Linear<3, Header, C>;

const _: () = assert!(core::mem::size_of::<Node3<()>>() == 64);
const _: () = assert!(core::mem::align_of::<Node3<()>>() == 64);

#[derive(Copy, Clone, Debug, ribbit::Pack)]
#[ribbit(size = 32, packed(rename = "HeaderPacked"), debug)]
pub(crate) struct Header {
    keys: u24,
    len: u4,
    frozen: bool,
}

impl Header {
    const DEFAULT: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(u24::new(0), u4::new(0), false);
}

impl Default for HeaderPacked {
    fn default() -> Self {
        Header::DEFAULT
    }
}

impl linear::Header for ribbit::Packed<Header> {
    const KIND: node::Kind = node::Kind::Node3;
    const GROW: usize = 3;

    type Grow<M>
        = Node15<M>
    where
        M: edge::Meta;
    type Shrink<M>
        = Node3<M>
    where
        M: edge::Meta;

    #[inline]
    fn freeze(self) -> Self {
        self.with_frozen(true)
    }

    #[inline]
    fn is_frozen(self) -> bool {
        self.frozen()
    }

    #[inline]
    fn len(self) -> usize {
        self.len().value() as usize
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
                u24::new(self.keys().value() | ((key as u32) << (len << 3))),
                u4::new(len + 1),
                false,
            ))),
        }
    }

    fn keys_range<L: crate::raw::node::Lower, H: crate::raw::node::Upper>(
        self,
        low: L,
        high: H,
    ) -> linear::KeyIter {
        let len = self.len().value() as usize;
        let keys = self.value.to_le_bytes();
        let mut valid = 0;
        let low = low.get();
        let high = high.get();

        let mut indexes: [(u8, u8); 3] = core::array::from_fn(|index| {
            if index < len && (low..=high).contains(&keys[index]) {
                valid += 1;
                (keys[index], index as u8)
            } else {
                (255, 3)
            }
        });

        indexes.sort_unstable();
        linear::KeyIter::new_3(linear::RawIter::new(indexes, valid))
    }

    fn keys_sorted(self) -> linear::KeyIter {
        let len = self.len().value();
        let keys = self.value.to_le_bytes();
        let mut indexes: [(u8, u8); 3] = core::array::from_fn(|index| (keys[index], index as u8));
        indexes[..len as usize].sort_unstable();
        linear::KeyIter::new_3(linear::RawIter::new(indexes, len))
    }

    fn keys_unsorted(self) -> linear::KeyIter {
        let len = self.len().value();
        let keys = self.value.to_le_bytes();
        let indexes: [(u8, u8); 3] = core::array::from_fn(|index| (keys[index], index as u8));
        linear::KeyIter::new_3(linear::RawIter::new(indexes, len))
    }
}

#[inline]
fn get(array: u32, key: u8) -> u8 {
    if cfg!(feature = "opt-node3-get") {
        get_swar(array, key)
    } else {
        get_naive(array, key)
    }
}

/// https://richardstartin.github.io/posts/finding-bytes
/// https://orlp.net/blog/extracting-depositing-bits/
#[inline]
fn get_swar(array: u32, key: u8) -> u8 {
    const LOWER: u32 = 0x00_7F_7F_7F;

    // LLVM is smart enough to turn this into an `imul`
    const fn broadcast(byte: u8) -> u32 {
        let byte = byte as u32;
        byte | (byte << 8) | (byte << 16)
    }

    let diff = array ^ broadcast(key);

    // Carry lower 7 bits of each byte into top bit
    let any_one_lower = (diff & LOWER) + LOWER;

    // Combine top bit of `diff` with carried bit
    let any_one = diff | any_one_lower;

    ((any_one | LOWER).trailing_ones() >> 3) as u8
}

#[inline]
fn get_naive(array: u32, key: u8) -> u8 {
    array
        .to_le_bytes()
        .into_iter()
        .position(|byte| byte == key)
        .map(|index| index as u8)
        .unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
    #[test]
    fn zero() {
        test_get(0x00_00_00_00, 0, 0)
    }

    #[test]
    fn zero_high() {
        test_get(0x00_00_12_34, 0, 2)
    }

    #[test]
    fn nonzero_middle() {
        test_get(0x00_11_12_13, 0x12, 1)
    }

    #[test]
    fn duplicate() {
        test_get(0x00_11_11_12, 0x11, 1)
    }

    fn test_get(array: u32, key: u8, expected: u8) {
        assert_eq!(super::get_naive(array, key), expected);
        assert_eq!(super::get_swar(array, key), expected);
    }
}
