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
        #[cfg(not(all(target_arch = "x86_64", target_feature = "sse2")))]
        compile_error!("opt-node15-get requires target_arch=x86_64 and target_feature=sse2");

        use core::arch::x86_64::_mm_cmpeq_epi8;
        use core::arch::x86_64::_mm_movemask_epi8;
        use core::arch::x86_64::_mm_set1_epi8;
        use std::arch::x86_64::__m128i;

        unsafe {
            _mm_movemask_epi8(_mm_cmpeq_epi8(
                core::mem::transmute::<u128, __m128i>(self.value()),
                _mm_set1_epi8(key as i8),
            ))
            .trailing_zeros() as usize
        }
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
    type Item = (u8, &'a Edge);
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
