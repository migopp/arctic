use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic64;
use ribbit::u24;
use ribbit::u4;

use crate::iter::Or;
use crate::node;
use crate::node::linear;

use super::Node15;

pub(crate) type Node3 = super::Linear<3, Atomic64<Header>>;

const _: () = assert!(core::mem::size_of::<Node3>() == 64);
const _: () = assert!(core::mem::align_of::<Node3>() == 64);

#[derive(Copy, Clone, Debug, Default, ribbit::Pack)]
#[ribbit(size = 32)]
pub(crate) struct Header {
    keys: u24,
    len: u4,
    frozen: bool,
}

impl linear::Header for Atomic64<Header> {
    fn freeze(&self) -> usize {
        let mut old = self.load_packed(Ordering::Relaxed);

        while !old.frozen() {
            match self.compare_exchange_packed(
                old,
                old.with_frozen(true),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(old) => return old.len().value() as usize,
                Err(conflict) => old = conflict,
            }
        }

        old.len().value() as usize
    }

    #[inline]
    fn get(&self, key: u8) -> Option<u8> {
        let header = self.load_packed(Ordering::Relaxed);
        let index = get(header.value, key);
        (index < header.len().value()).then_some(index)
    }

    #[inline]
    fn get_or_reserve(&self, key: u8) -> Option<u8> {
        let mut old = self.load_packed(Ordering::Acquire);

        loop {
            let index = get(old.value, key);
            let len = old.len().value();

            if index < len {
                return Some(index);
            } else if len >= 3 || old.frozen() {
                return None;
            }

            match self.compare_exchange_packed(
                old,
                ribbit::Packed::<Header>::new(
                    u24::new(old.keys().value() | ((key as u32) << (len * 8))),
                    u4::new(len + 1),
                    false,
                ),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(len),
                Err(conflict) => old = conflict,
            }
        }
    }

    #[inline]
    fn keys_range(&self, min: u8, max: u8) -> linear::RangeKeyIter {
        let header = self.load_packed(Ordering::Relaxed);
        let mut len = header.len().value() as usize;
        let keys = header.value.to_ne_bytes();

        let mut indexes: [(u8, u8); 3] = core::array::from_fn(|index| {
            if index < len {
                if (min..=max).contains(&keys[index]) {
                    return (keys[index], index as u8);
                } else {
                    len -= 1;
                }
            }

            (255, 3)
        });

        indexes.sort_unstable();
        Or::L(indexes.into_iter().take(len))
    }

    #[inline]
    fn keys_sorted(&self) -> linear::SortedKeyIter {
        let header = self.load_packed(Ordering::Relaxed);
        let len = header.len().value() as usize;
        let keys = header.value.to_ne_bytes();
        let mut indexes: [(u8, u8); 3] = core::array::from_fn(|index| (keys[index], index as u8));
        indexes[..len].sort_unstable();
        Or::L(indexes.into_iter().take(len))
    }

    #[inline]
    fn keys_unsorted(&self) -> linear::UnsortedKeyIter {
        let header = self.load_packed(Ordering::Relaxed);
        Or::L(
            header
                .value
                .to_ne_bytes()
                .into_iter()
                .take(header.len().value() as usize),
        )
    }
}

impl node::Info for Node3 {
    const KIND: node::Kind = node::Kind::Node3;
    const GROW: usize = 3;
    const REF: for<'a> fn(&'a Self) -> node::Ref<'a> = |node| node::Ref::Node3(node);

    type Grow = Node15;
    type Shrink = Node3;
}

/// https://richardstartin.github.io/posts/finding-bytes
/// https://orlp.net/blog/extracting-depositing-bits/
#[inline]
#[cfg(feature = "opt-node3-get")]
fn get(array: u32, key: u8) -> u8 {
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
#[cfg(not(feature = "opt-node3-get"))]
fn get(array: u32, key: u8) -> u8 {
    linear::UnsortedKeyIter::new_3(array, 3)
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
    fn first() {
        test_get(0x00_11_11_12, 0x11, 1)
    }

    fn test_get(array: u32, key: u8, expected: u8) {
        assert_eq!(super::get(array, key), expected);
    }
}
