use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;
use ribbit::atomic::Atomic64;
use ribbit::u24;
use ribbit::u4;

use crate::node;
use crate::node::linear;
use crate::Edge;

use super::linear::Header as _;
use super::Node15;

pub(crate) type Node3 = super::Linear<3, Atomic64<Header>>;

const _: () = assert!(core::mem::size_of::<Node3>() == 64);
const _: () = assert!(core::mem::align_of::<Node3>() == 64);

#[derive(Copy, Clone, Debug, Default)]
#[ribbit::pack(size = 32)]
pub(crate) struct Header {
    keys: u24,
    len: u4,
    frozen: bool,
}

impl linear::Header for Atomic64<Header> {
    fn is_frozen(&self) -> bool {
        self.load_packed(Ordering::Relaxed).frozen()
    }

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

    fn keys(&self) -> super::KeyIter {
        super::KeyIter::new_3(self.load_packed(Ordering::Relaxed).value)
    }
}

impl<'a> IntoIterator for &'a Node3 {
    type Item = (u8, &'a Atomic128<Edge>);
    type IntoIter = super::Iter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        self.header.keys().zip(self.edges.as_slice().iter())
    }
}

impl node::Info for Node3 {
    const KIND: ribbit::Packed<node::Kind> = ribbit::Packed::<node::Kind>::new_node3();
    const GROW: usize = 3;
    const REF: for<'a> fn(&'a Self) -> node::Ref<'a> = |node| node::Ref::Node3(node);

    type Grow = Node15;
    type Shrink = Node3;
}

#[inline]
#[cfg(feature = "opt-node3-get")]
fn get(array: u32, key: u8) -> u8 {
    // https://richardstartin.github.io/posts/finding-bytes
    const PATTERN: u32 = 0x7F_7F_7F_7F;

    const fn broadcast(byte: u8) -> u32 {
        let byte = byte as u32;
        byte | (byte << 8) | (byte << 16)
    }

    let input = array ^ broadcast(key);
    let temp = (input & PATTERN) + PATTERN;
    let temp = !(input | temp | PATTERN);

    (temp.trailing_zeros() >> 3) as u8
}

#[inline]
#[cfg(not(feature = "opt-node3-get"))]
fn get(array: u32, key: u8) -> u8 {
    super::KeyIter::new_3(array)
        .position(|byte| byte == key)
        .map(|index| index as u8)
        .unwrap_or(u8::MAX)
}
