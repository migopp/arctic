use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;
use ribbit::u120;
use ribbit::u4;

use crate::node;
use crate::node::linear;
use crate::node::Node256;
use crate::node::Node3;

pub(crate) type Node15 = super::Linear<15, Atomic128<Header>>;

const _: () = assert!(core::mem::size_of::<Node15>() == 256);
const _: () = assert!(core::mem::align_of::<Node15>() == 64);

#[derive(Copy, Clone, Debug, Default, ribbit::Pack)]
#[ribbit(size = 128)]
pub(crate) struct Header {
    keys: u120,
    len: u4,
    frozen: bool,
}

impl linear::Header for Atomic128<Header> {
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
            } else if len >= 15 || old.frozen() {
                return None;
            }

            match self.compare_exchange_packed(
                old,
                ribbit::Packed::<Header>::new(
                    u120::new(old.keys().value() | ((key as u128) << (len * 8))),
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

    fn keys_sorted(&self) -> linear::SortedKeyIter {
        let header = self.load_packed(Ordering::Relaxed);
        linear::SortedKeyIter::new_15(header.value, header.len().value() as usize)
    }

    fn keys_unsorted(&self) -> linear::UnsortedKeyIter {
        let header = self.load_packed(Ordering::Relaxed);
        linear::UnsortedKeyIter::new_15(header.value, header.len().value() as usize)
    }
}

impl node::Info for Node15 {
    const KIND: ribbit::Packed<node::Kind> = ribbit::Packed::<node::Kind>::new_node15();
    const GROW: usize = 15;
    const REF: for<'a> fn(&'a Self) -> node::Ref<'a> = |node| node::Ref::Node15(node);

    type Grow = Node256;
    type Shrink = Node3;
}

#[inline]
#[cfg(feature = "opt-node15-get")]
fn get(array: u128, key: u8) -> u8 {
    #[cfg(not(all(target_arch = "x86_64", target_feature = "sse2")))]
    compile_error!("opt-node15-get requires target_arch=x86_64 and target_feature=sse2");

    use core::arch::x86_64::_mm_cmpeq_epi8;
    use core::arch::x86_64::_mm_movemask_epi8;
    use core::arch::x86_64::_mm_set1_epi8;
    use std::arch::x86_64::__m128i;

    unsafe {
        _mm_movemask_epi8(_mm_cmpeq_epi8(
            core::mem::transmute::<u128, __m128i>(array),
            _mm_set1_epi8(key as i8),
        ))
        .trailing_zeros() as u8
    }
}

#[inline]
#[cfg(not(feature = "opt-node15-get"))]
fn get(array: u128, key: u8) -> u8 {
    linear::UnsortedKeyIter::new_15(array, 15)
        .position(|byte| byte == key)
        .map(|index| index as u8)
        .unwrap_or(u8::MAX)
}
