use core::sync::atomic::Ordering;

use ribbit::u120;

use crate::node;
use crate::node::linear;
use crate::node::Edge;
use crate::node::Node256;
use crate::node::Node3;

pub(crate) type Node15 = super::Linear<15, u120>;

const _: () = assert!(core::mem::size_of::<Node15>() == 256);

impl linear::KeyArray for u120 {
    const LEN: usize = 15;

    #[cfg(feature = "opt-node15-get")]
    fn get(&self, key: u8) -> usize {
        // https://richardstartin.github.io/posts/finding-bytes
        const PATTERN: u128 = {
            let mut pattern = 0;
            let mut i = 0;
            while i < 16 {
                pattern |= 0x7Fu128 << i;
                i += 1;
            }
            pattern
        };

        const fn broadcast(byte: u8) -> u128 {
            let byte = byte as u128;
            byte | (byte << 8)
                | (byte << 16)
                | (byte << 24)
                | (byte << 32)
                | (byte << 40)
                | (byte << 48)
                | (byte << 56)
                | (byte << 64)
                | (byte << 72)
                | (byte << 80)
                | (byte << 88)
                | (byte << 96)
                | (byte << 104)
                | (byte << 112)
                | (byte << 120)
        }

        let input = self.value() ^ broadcast(key);
        let temp = (input & PATTERN) + PATTERN;
        let temp = !(input | temp | PATTERN);
        (temp.trailing_zeros() >> 3) as usize
    }

    fn insert(&self, index: usize, key: u8) -> Self {
        let mut keys = self.value();
        keys |= (key as u128) << (index * 8);
        Self::new(keys)
    }

    fn iter(&self) -> impl Iterator<Item = u8> {
        super::KeyIter::new_15(*self)
    }
}

impl<'a> IntoIterator for &'a Node15 {
    type Item = (u8, Edge);
    type IntoIter = super::Iter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        let header = self.header.load(Ordering::Relaxed);
        super::KeyIter::new_15(header.keys).zip(super::EdgeIter::new(&self.edges))
    }
}

impl node::Info for Node15 {
    const KIND: node::Kind = node::Kind::Node15;
    const GROW: usize = 15;

    type Grow = Node256;
    type Shrink = Node3;
}
