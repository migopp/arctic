macro_rules! validate {
    ($($tt:tt)*) => {
        if cfg!(any(feature = "validate", debug_assertions)) {
            assert!($($tt)*);
        }
    };
}

macro_rules! validate_eq {
    ($($tt:tt)*) => {
        if cfg!(any(feature = "validate", debug_assertions)) {
            assert_eq!($($tt)*);
        }
    };
}

mod cursor;
mod edge;
mod key;
mod node;
mod raw;
pub mod stat;

pub use raw::Raw;
use ribbit::u3;

use core::marker::PhantomData;
use core::ops::RangeBounds;

pub(crate) use edge::Edge;
pub(crate) use node::Node;

pub struct Map<K: ?Sized, V> {
    raw: Raw<K>,
    _key: PhantomData<K>,
    _value: PhantomData<V>,
}

impl<K: ?Sized, V> Default for Map<K, V> {
    fn default() -> Self {
        Self {
            raw: Raw::default(),
            _key: PhantomData,
            _value: PhantomData,
        }
    }
}

impl<K: Key + ?Sized, V: Value> Map<K, V> {
    pub fn get(&self, key: &K) -> Option<V> {
        key.with_swap(|key| self.raw.get(key).map(V::from_u64))
    }

    pub fn insert(&self, key: &K, value: V) -> Option<V> {
        key.with_swap(|key| self.raw.insert(key, value.into_u64()).map(V::from_u64))
    }

    pub fn remove(&self, key: &K) -> Option<V> {
        key.with_swap(|key| self.raw.remove(key).map(V::from_u64))
    }

    pub fn update(&self, key: &K, value: V) -> Option<V> {
        key.with_swap(|key| self.raw.update(key, value.into_u64()).map(V::from_u64))
    }

    // pub fn iter(&mut self) -> impl Iterator<Item = (K::Owned, V)> + '_ {
    //     self.raw
    //         .iter()
    //         .map(|(key, value)| (K::from_byte_array(key), V::from_u64(value)))
    // }
    //
    // pub fn keys(&mut self) -> impl Iterator<Item = K::Owned> + '_ {
    //     self.iter().map(|(key, _)| key)
    // }
    //
    // pub fn values(&mut self) -> impl Iterator<Item = V> + '_ {
    //     self.iter().map(|(_, value)| value)
    // }
    //
    // pub fn scan(&self, low: &K, count: usize) -> impl Iterator<Item = V> {
    //     self.raw.scan(low, count).map(V::from_u64)
    // }
    //
    // pub fn range<'a, R: RangeBounds<&'a K> + 'a>(&self, range: R) -> impl Iterator<Item = V> + 'a
    // where
    //     K: 'a,
    //     V: 'a,
    // {
    //     let low = range.start_bound().map(|low| low.to_byte_array());
    //     let high = range.end_bound().map(|high| high.to_byte_array());
    //     self.raw.range((low, high)).map(V::from_u64)
    // }
}

pub trait Key {
    #[inline]
    fn with_swap<F: FnOnce(&Self) -> T, T>(&self, with: F) -> T {
        with(self)
    }

    fn len(&self) -> usize;

    #[inline]
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    fn get(&self, index: usize) -> Option<u8> {
        (index < self.len()).then(|| unsafe { self.get_unchecked(index) })
    }

    /// # SAFETY
    ///
    /// Caller must guarantee `index < self.len()`.
    unsafe fn get_unchecked(&self, index: usize) -> u8;

    /// Return `len` bytes starting from `index` in least significant 7 bytes.
    ///
    /// # SAFETY
    ///
    /// Caller must guarantee:
    /// - `index <= self.len()`
    /// - `index + len <= self.len()`
    unsafe fn get_array_unchecked(&self, index: usize, len: u3) -> u64;
}

impl Key for u8 {
    #[inline]
    fn len(&self) -> usize {
        1
    }

    #[inline]
    unsafe fn get_unchecked(&self, _index: usize) -> u8 {
        *self
    }

    #[inline]
    unsafe fn get_array_unchecked(&self, _index: usize, len: u3) -> u64 {
        match len.value() {
            0 => 0u64,
            _ => *self as u64,
        }
    }
}

impl Key for u64 {
    #[inline]
    #[cfg(target_endian = "little")]
    fn with_swap<F: FnOnce(&Self) -> T, T>(&self, with: F) -> T {
        with(&self.swap_bytes())
    }

    #[inline]
    fn len(&self) -> usize {
        8
    }

    #[inline]
    unsafe fn get_unchecked(&self, index: usize) -> u8 {
        (self >> (index << 3)) as u8
    }

    #[inline]
    unsafe fn get_array_unchecked(&self, index: usize, len: u3) -> u64 {
        if len.value() == 0 {
            0
        } else {
            (self >> (index << 3)) & ((1 << ((len.value() as u64) << 3)) - 1)
        }
    }
}

impl Key for str {
    #[inline]
    fn len(&self) -> usize {
        (*self).len()
    }

    #[inline]
    unsafe fn get_unchecked(&self, index: usize) -> u8 {
        *self.as_bytes().get_unchecked(index)
    }

    #[inline]
    unsafe fn get_array_unchecked(&self, index: usize, len: u3) -> u64 {
        let mut buffer = [0u8; 8];
        let len = len.value() as usize;
        buffer[..len].copy_from_slice(&self.as_bytes()[index..][..len]);
        u64::from_ne_bytes(buffer)
    }
}

impl Key for String {
    #[inline]
    fn len(&self) -> usize {
        (*self).len()
    }

    #[inline]
    unsafe fn get_unchecked(&self, index: usize) -> u8 {
        *self.as_bytes().get_unchecked(index)
    }

    #[inline]
    unsafe fn get_array_unchecked(&self, index: usize, len: u3) -> u64 {
        let mut buffer = [0u8; 8];
        let len = len.value() as usize;
        buffer[..len].copy_from_slice(&self.as_bytes()[index..][..len]);
        u64::from_ne_bytes(buffer)
    }
}

impl Key for [u8] {
    #[inline]
    fn len(&self) -> usize {
        self.len()
    }

    #[inline]
    unsafe fn get_unchecked(&self, index: usize) -> u8 {
        *self.get_unchecked(index)
    }

    #[inline]
    unsafe fn get_array_unchecked(&self, index: usize, len: u3) -> u64 {
        let mut buffer = [0u8; 8];
        let len = len.value() as usize;
        buffer[..len].copy_from_slice(&self[index..][..len]);
        u64::from_ne_bytes(buffer)
    }
}

impl<const N: usize> Key for [u8; N] {
    #[inline]
    fn len(&self) -> usize {
        N
    }

    #[inline]
    unsafe fn get_unchecked(&self, index: usize) -> u8 {
        *<[u8]>::get_unchecked(self, index)
    }

    #[inline]
    unsafe fn get_array_unchecked(&self, index: usize, len: u3) -> u64 {
        let mut buffer = [0u8; 8];
        let len = len.value() as usize;
        buffer[..len].copy_from_slice(&self[index..][..len]);
        u64::from_ne_bytes(buffer)
    }
}

pub trait Value {
    fn from_u64(value: u64) -> Self;
    fn into_u64(self) -> u64;
}

impl Value for u32 {
    #[inline]
    fn from_u64(value: u64) -> Self {
        value as u32
    }

    #[inline]
    fn into_u64(self) -> u64 {
        self as u64
    }
}

impl Value for () {
    #[inline]
    fn from_u64(_: u64) -> Self {}

    #[inline]
    fn into_u64(self) -> u64 {
        0
    }
}

#[derive(Debug)]
enum Or<L, R> {
    L(L),
    R(R),
}

impl<L, R, T> Iterator for Or<L, R>
where
    L: Iterator<Item = T>,
    R: Iterator<Item = T>,
{
    type Item = T;
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Or::L(left) => left.next(),
            Or::R(right) => right.next(),
        }
    }
}

impl<L, R> Or<L, R>
where
    L: Iterator,
    R: Iterator,
{
    fn skip(&mut self) {
        match self {
            Or::L(left) => {
                left.next();
            }
            Or::R(right) => {
                right.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Map;

    #[test]
    fn smoke() {
        let map = Map::<[u8], _>::default();
        map.insert(b"abcd", 1);
        assert_eq!(map.get(b"abcd"), Some(1));
    }

    #[test]
    fn smoke_u64_key() {
        let map = Map::default();
        let key = 0xdeadbeefu64.to_be_bytes();
        map.insert(&key, 1);
        assert_eq!(map.get(&key), Some(1));
    }
    //
    // #[test]
    // fn scan_leaf() {
    //     let map = Map::default();
    //     let key = [1];
    //     map.insert(&key, 1);
    //     assert_eq!(map.range(&[1]..=&[1]).collect::<Vec<_>>(), vec![1]);
    // }
    //
    // #[test]
    // fn scan_node3() {
    //     let map = insert_all(0u64..3);
    //     assert_eq!(
    //         map.range(&0..=&2).collect::<Vec<_>>(),
    //         (0..3).collect::<Vec<_>>()
    //     );
    // }
    //
    // #[test]
    // fn scan_node256() {
    //     let map = insert_all(0u64..256);
    //     assert_eq!(
    //         map.range(&0..=&255).collect::<Vec<_>>(),
    //         (0..256).collect::<Vec<_>>()
    //     );
    // }
    //
    // #[test]
    // fn scan_node256_exclusive() {
    //     let map = insert_all(0u64..256);
    //     assert_eq!(
    //         map.range(&0..&256).collect::<Vec<_>>(),
    //         (0..256).collect::<Vec<_>>()
    //     );
    // }
    //
    // #[test]
    // fn scan_gap() {
    //     let map = insert_all((0u64..512).step_by(2));
    //     assert_eq!(
    //         map.range(&256..=&511).collect::<Vec<_>>(),
    //         (128..256).collect::<Vec<_>>()
    //     );
    // }
    //
    // #[test]
    // fn node3_overwrite() {
    //     let mut map = Map::default();
    //
    //     for value in [1, 2, 3] {
    //         map.insert(&1u8, value);
    //         assert_eq!(map.get(&1), Some(value));
    //     }
    //
    //     assert_eq!(map.iter().count(), 1);
    //
    //     map.iter().for_each(|(key, value)| {
    //         assert_eq!(key, 1);
    //         assert_eq!(value, 3);
    //     });
    // }

    #[test]
    fn node3_full() {
        insert_all(0u8..3);
    }

    #[test]
    fn node3_expand() {
        insert_all(0u8..4);
    }

    #[test]
    fn node15_full() {
        insert_all(0u8..15);
    }

    #[test]
    fn node15_expand() {
        insert_all(0u8..16);
    }

    #[test]
    fn node256_full() {
        insert_all(0u8..=255);
    }

    fn insert_all<I, K>(iter: I) -> Map<K, u32>
    where
        I: IntoIterator<Item = K>,
        I::IntoIter: Clone,
        K: crate::Key + Clone + Ord + core::fmt::Debug,
    {
        let keys = iter
            .into_iter()
            .enumerate()
            .map(|(index, key)| (key, index as u32));

        let map = Map::default();

        for (key, value) in keys.clone() {
            map.insert(&key, value);
            assert_eq!(map.get(&key), Some(value));
        }

        for (key, value) in keys.clone() {
            assert_eq!(map.get(&key), Some(value));
        }

        // assert_eq!(map.iter().count(), keys.clone().count());
        //
        // let mut sorted = keys.collect::<Vec<_>>();
        // sorted.sort_by(|(l, _), (r, _)| l.cmp(r));
        //
        // sorted
        //     .into_iter()
        //     .zip(map.iter())
        //     .for_each(|((lk, lv), (rk, rv))| {
        //         assert_eq!(lk, rk);
        //         assert_eq!(lv, rv);
        //     });

        map
    }
}
