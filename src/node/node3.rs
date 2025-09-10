use core::sync::atomic::Ordering;

use ribbit::u24;

use crate::Edge;

pub(crate) type Node3 = super::Linear<3, u24>;

const _: () = assert!(core::mem::size_of::<Node3>() == 64);

impl super::linear::KeyArray for u24 {
    #[cfg(feature = "opt-node3-get")]
    fn get(&self, key: u8) -> usize {
        // https://richardstartin.github.io/posts/finding-bytes
        const PATTERN: u32 = 0x7F_7F_7F_7F;

        const fn broadcast(byte: u8) -> u32 {
            let byte = byte as u32;
            byte | (byte << 8) | (byte << 16) | (byte << 24)
        }

        let input = self.value() ^ broadcast(key);
        let temp = (input & PATTERN) + PATTERN;
        let temp = !(input | temp | PATTERN);

        (temp.trailing_zeros() >> 3) as usize
    }

    fn insert(&self, index: usize, key: u8) -> Self {
        let mut keys = self.value();
        keys |= (key as u32) << (index * 8);
        Self::new(keys)
    }

    fn iter(&self) -> impl Iterator<Item = u8> {
        let keys = self.value();
        (0..3).map(move |i| (keys >> (i * 8)) as u8)
    }
}

impl<'a> IntoIterator for &'a Node3 {
    type Item = (Option<u8>, Edge);
    type IntoIter = super::Iter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        let header = self.header.load(Ordering::Relaxed);
        super::KeyIter::new_3(header.keys).zip(super::EdgeIter::new(&self.edges))
    }
}
