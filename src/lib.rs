mod cursor;
mod edge;
mod key;
mod node;
mod raw;
mod split;
pub mod stat;

pub use raw::Raw;

use core::marker::PhantomData;
use core::ops::RangeBounds;
use std::rc::Rc;

pub(crate) use edge::Edge;
pub(crate) use node::Node;
pub(crate) use split::Split;

pub struct Map<K, V> {
    raw: Raw,
    _key: PhantomData<K>,
    _value: PhantomData<V>,
}

impl<K, V> Default for Map<K, V> {
    fn default() -> Self {
        Self {
            raw: Raw::default(),
            _key: PhantomData,
            _value: PhantomData,
        }
    }
}

impl<K: Key, V: Value> Map<K, V> {
    pub fn get(&self, key: &K) -> Option<V> {
        let key = key.to_byte_array();
        let key = key.as_ref();
        self.raw.get(key).map(V::from_u64)
    }

    pub fn insert(&self, key: &K, value: V) -> Option<V> {
        let key = key.to_byte_array();
        let key = key.as_ref();
        self.raw.insert(key, value.into_u64()).map(V::from_u64)
    }

    pub fn remove(&self, key: &K) -> Option<V> {
        let key = key.to_byte_array();
        let key = key.as_ref();
        self.raw.remove(key).map(V::from_u64)
    }

    pub fn update(&self, key: &K, value: V) -> Option<V> {
        let key = key.to_byte_array();
        let key = key.as_ref();
        self.raw.update(key, value.into_u64()).map(V::from_u64)
    }

    pub fn iter(&mut self) -> impl Iterator<Item = (K::Owned, V)> + '_ {
        self.raw
            .iter()
            .map(|(key, value)| (K::from_byte_array(key), V::from_u64(value)))
    }

    pub fn keys(&mut self) -> impl Iterator<Item = K::Owned> + '_ {
        self.iter().map(|(key, _)| key)
    }

    pub fn values(&mut self) -> impl Iterator<Item = V> + '_ {
        self.iter().map(|(_, value)| value)
    }

    pub fn range<'a, R: RangeBounds<&'a K> + 'a>(&self, range: R) -> impl Iterator<Item = V> + 'a
    where
        K: 'a,
        V: 'a,
    {
        let low = range.start_bound().map(|low| low.to_byte_array());
        let high = range.end_bound().map(|high| high.to_byte_array());
        self.raw.range((low, high)).map(V::from_u64)
    }
}

// TODO: add size hint for iterator key buffer? or use arrayvec?
pub trait Key {
    type ByteArray<'a>: AsRef<[u8]>
    where
        Self: 'a;
    type Owned;

    fn to_byte_array(&self) -> Self::ByteArray<'_>;
    // TODO: avoid cloning?
    fn from_byte_array(array: Rc<Vec<u8>>) -> Self::Owned;
}

macro_rules! impl_key {
    ($($type:ident: $len:expr),* $(,)?) => {
        $(
            impl Key for $type {
                type ByteArray<'a> = [u8; $len];
                type Owned = Self;

                fn to_byte_array(&self) -> Self::ByteArray<'static> {
                    self.to_be_bytes()
                }

                fn from_byte_array(array: Rc<Vec<u8>>) -> Self {
                    Self::from_be_bytes(Self::ByteArray::try_from(array.as_slice()).unwrap())
                }
            }
        )*
    };
}

impl_key!(
    u64: 8,
    u32: 4,
    u16: 2,
    u8: 1,
);

impl Key for &'_ str {
    type ByteArray<'a>
        = &'a [u8]
    where
        Self: 'a;
    type Owned = String;

    #[inline]
    fn to_byte_array(&self) -> Self::ByteArray<'_> {
        self.as_bytes()
    }

    #[inline]
    fn from_byte_array(array: Rc<Vec<u8>>) -> Self::Owned {
        String::from_utf8(Rc::unwrap_or_clone(array)).unwrap()
    }
}

impl Key for String {
    type ByteArray<'a> = &'a [u8];
    type Owned = String;

    #[inline]
    fn to_byte_array(&self) -> Self::ByteArray<'_> {
        self.as_bytes()
    }

    #[inline]
    fn from_byte_array(array: Rc<Vec<u8>>) -> Self::Owned {
        String::from_utf8(Rc::unwrap_or_clone(array)).unwrap()
    }
}

impl Key for &'_ [u8] {
    type ByteArray<'a>
        = &'a [u8]
    where
        Self: 'a;
    type Owned = Vec<u8>;

    #[inline]
    fn to_byte_array(&self) -> Self::ByteArray<'_> {
        self
    }

    #[inline]
    fn from_byte_array(array: Rc<Vec<u8>>) -> Self::Owned {
        Rc::unwrap_or_clone(array)
    }
}

impl<const LEN: usize> Key for [u8; LEN] {
    type ByteArray<'a> = Self;
    type Owned = Self;

    #[inline]
    fn to_byte_array(&self) -> Self::ByteArray<'static> {
        *self
    }

    #[inline]
    fn from_byte_array(array: Rc<Vec<u8>>) -> Self::Owned {
        Rc::unwrap_or_clone(array).try_into().unwrap()
    }
}

impl<const LEN: usize> Key for &'_ [u8; LEN] {
    type ByteArray<'a>
        = &'a [u8; LEN]
    where
        Self: 'a;
    type Owned = [u8; LEN];

    #[inline]
    fn to_byte_array(&self) -> Self::ByteArray<'_> {
        self
    }

    #[inline]
    fn from_byte_array(array: Rc<Vec<u8>>) -> Self::Owned {
        Rc::unwrap_or_clone(array).try_into().unwrap()
    }
}

impl Key for Vec<u8> {
    type ByteArray<'a> = &'a [u8];
    type Owned = Self;

    #[inline]
    fn to_byte_array(&self) -> Self::ByteArray<'_> {
        self
    }

    #[inline]
    fn from_byte_array(array: Rc<Vec<u8>>) -> Self::Owned {
        Rc::unwrap_or_clone(array)
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

#[cfg(test)]
mod tests {
    use crate::Map;

    #[test]
    fn smoke() {
        let map = Map::default();
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

    #[test]
    fn scan_leaf() {
        let map = Map::default();
        let key = [1];
        map.insert(&key, 1);
        assert_eq!(map.range(&[1]..=&[1]).collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn scan_node3() {
        let map = insert_all(0u64..3);
        assert_eq!(
            map.range(&0..=&2).collect::<Vec<_>>(),
            (0..3).collect::<Vec<_>>()
        );
    }

    #[test]
    fn scan_node256() {
        let map = insert_all(0u64..256);
        assert_eq!(
            map.range(&0..=&255).collect::<Vec<_>>(),
            (0..256).collect::<Vec<_>>()
        );
    }

    #[test]
    fn scan_node256_exclusive() {
        let map = insert_all(0u64..256);
        assert_eq!(
            map.range(&0..&256).collect::<Vec<_>>(),
            (0..256).collect::<Vec<_>>()
        );
    }

    #[test]
    fn scan_gap() {
        let map = insert_all((0u64..512).step_by(2));
        assert_eq!(
            map.range(&256..=&511).collect::<Vec<_>>(),
            (128..256).collect::<Vec<_>>()
        );
    }

    #[test]
    fn node3_overwrite() {
        let mut map = Map::default();

        for value in [1, 2, 3] {
            map.insert(&1u8, value);
            assert_eq!(map.get(&1), Some(value));
        }

        assert_eq!(map.iter().count(), 1);

        map.iter().for_each(|(key, value)| {
            assert_eq!(key, 1);
            assert_eq!(value, 3);
        });
    }

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
        K: crate::Key + Clone + Ord + PartialEq<K::Owned> + core::fmt::Debug,
        K::Owned: core::fmt::Debug,
    {
        let keys = iter
            .into_iter()
            .enumerate()
            .map(|(index, key)| (key, index as u32));

        let mut map = Map::default();

        for (key, value) in keys.clone() {
            map.insert(&key, value);
            assert_eq!(map.get(&key), Some(value));
        }

        for (key, value) in keys.clone() {
            assert_eq!(map.get(&key), Some(value));
        }

        assert_eq!(map.iter().count(), keys.clone().count());

        let mut sorted = keys.collect::<Vec<_>>();
        sorted.sort_by(|(l, _), (r, _)| l.cmp(r));

        sorted
            .into_iter()
            .zip(map.iter())
            .for_each(|((lk, lv), (rk, rv))| {
                assert_eq!(lk, rk);
                assert_eq!(lv, rv);
            });

        map
    }
}
