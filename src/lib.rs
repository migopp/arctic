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

mod byte;
mod cursor;
mod edge;
#[cfg(feature = "smr-hazard")]
mod membarrier;
mod node;
mod raw;
mod smr;
pub mod stat;

pub(crate) use raw::Raw;

use core::marker::PhantomData;

pub(crate) use edge::Edge;
pub(crate) use node::Node;

pub struct Map<K: ?Sized, V> {
    raw: Raw,
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
    #[inline]
    pub fn pin(&self) -> MapRef<K, V> {
        MapRef {
            raw: self.raw.pin(),
            _key: PhantomData,
            _value: PhantomData,
        }
    }

    pub fn iter_ref(&mut self) -> Iter<K, V> {
        Iter {
            inner: self.raw.preorder(),
            _key: PhantomData,
            _value: PhantomData,
        }
    }
}

pub struct Iter<'a, K: Key + ?Sized, V> {
    inner: raw::iter::EntryIter<'a, K::Stack, raw::iter::SelectLeaf>,
    _key: PhantomData<K>,
    _value: PhantomData<V>,
}

impl<'a, K: Key + ?Sized, V: Value> Iter<'a, K, V> {
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<(&K::Stack, V)> {
        self.inner
            .next()
            .map(|(key, value)| (key, V::from_u64(value)))
    }
}

impl<K, V> Map<K, V>
where
    K: Key + for<'s> From<&'s K::Stack>,
    V: Value,
{
    pub fn iter(&mut self) -> impl Iterator<Item = (K, V)> + '_ {
        EntryIter {
            inner: self.raw.preorder(),
            _key: PhantomData,
            _value: PhantomData,
        }
    }
}

pub struct EntryIter<'a, K: Key, V> {
    inner: raw::iter::EntryIter<'a, K::Stack, raw::iter::SelectLeaf>,
    _key: PhantomData<K>,
    _value: PhantomData<V>,
}

impl<'a, K, V> Iterator for EntryIter<'a, K, V>
where
    K: Key,
    K: for<'s> From<&'s K::Stack>,
    V: Value,
{
    type Item = (K, V);
    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|(key, value)| (K::from(key), V::from_u64(value)))
    }
}

pub struct MapRef<'a, K: ?Sized, V> {
    raw: raw::Ref<'a>,
    _key: PhantomData<K>,
    _value: PhantomData<V>,
}

impl<K: Key + ?Sized, V: Value> MapRef<'_, K, V> {
    pub fn get(&self, key: &K) -> Option<V> {
        self.raw.get(key.iter()).map(V::from_u64)
    }

    pub fn insert(&mut self, key: &K, value: V) -> Option<V> {
        self.raw
            .insert(key.iter(), value.into_u64())
            .map(V::from_u64)
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        self.raw.remove(key.iter()).map(V::from_u64)
    }

    pub fn update(&mut self, key: &K, value: V) -> Option<V> {
        self.raw
            .update(key.iter(), value.into_u64())
            .map(V::from_u64)
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
    #[allow(private_bounds)]
    type Iter<'a>: byte::Iterator
    where
        Self: 'a;

    #[allow(private_bounds)]
    type Stack: byte::Stack;

    fn iter<'a>(&'a self) -> Self::Iter<'a>;
}

impl Key for u8 {
    type Iter<'a> = byte::fixed::Iter;
    type Stack = byte::fixed::Iter;
    #[inline]
    fn iter<'a>(&'a self) -> Self::Iter<'a> {
        byte::fixed::Iter::from(*self)
    }
}

impl Key for u64 {
    type Iter<'a> = byte::fixed::Iter;
    type Stack = byte::fixed::Iter;
    #[inline]
    fn iter<'a>(&'a self) -> Self::Iter<'a> {
        byte::fixed::Iter::from(*self)
    }
}

impl<const N: usize> Key for [u8; N] {
    type Iter<'a> = byte::dynamic::Iter<'a>;
    type Stack = Vec<u8>;
    #[inline]
    fn iter<'a>(&'a self) -> Self::Iter<'a> {
        byte::dynamic::Iter::from(self.as_slice())
    }
}

impl Key for [u8] {
    type Iter<'a> = byte::dynamic::Iter<'a>;
    type Stack = Vec<u8>;
    #[inline]
    fn iter<'a>(&'a self) -> Self::Iter<'a> {
        byte::dynamic::Iter::from(self)
    }
}

impl Key for Vec<u8> {
    type Iter<'a> = byte::dynamic::Iter<'a>;
    type Stack = Vec<u8>;
    #[inline]
    fn iter<'a>(&'a self) -> Self::Iter<'a> {
        byte::dynamic::Iter::from(self.as_slice())
    }
}

impl Key for str {
    type Iter<'a> = byte::dynamic::Iter<'a>;
    type Stack = Vec<u8>;
    #[inline]
    fn iter<'a>(&'a self) -> Self::Iter<'a> {
        byte::dynamic::Iter::from(self.as_bytes())
    }
}

impl Key for String {
    type Iter<'a> = byte::dynamic::Iter<'a>;
    type Stack = Vec<u8>;
    #[inline]
    fn iter<'a>(&'a self) -> Self::Iter<'a> {
        byte::dynamic::Iter::from(self.as_bytes())
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

/// https://users.rust-lang.org/t/compiler-hint-for-unlikely-likely-for-if-branches/62102/4
#[inline]
#[cold]
pub(crate) fn cold() {}

#[cfg(test)]
mod tests {
    use crate::Map;

    #[test]
    fn smoke() {
        let map = Map::<[u8], _>::default();
        let mut map = map.pin();
        map.insert(b"abcd", 1);
        assert_eq!(map.get(b"abcd"), Some(1));
    }

    #[test]
    fn smoke_u64_key() {
        let map = Map::default();
        let key = 0xdeadbeefu64.to_be_bytes();
        let mut map = map.pin();
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

    #[test]
    fn node3_overwrite() {
        let mut map = Map::default();
        let mut pin = map.pin();

        for value in [1, 2, 3] {
            pin.insert(&1u8, value);
            assert_eq!(pin.get(&1), Some(value));
        }

        drop(pin);
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

    #[test]
    fn split_edges() {
        let mut key = (0..100).collect::<Vec<_>>();
        insert_all(core::iter::from_fn(|| {
            if key.is_empty() {
                None
            } else {
                let mut next = key.clone();
                next.push(0);
                key.pop();
                Some(next)
            }
        }));
    }

    fn insert_all<I, K>(iter: I) -> Map<K, u32>
    where
        I: IntoIterator<Item = K>,
        K: crate::Key + Clone + Ord + core::fmt::Debug,
        K::Stack: PartialEq<K>,
    {
        let mut keys = iter
            .into_iter()
            .enumerate()
            .map(|(index, key)| (key, index as u32))
            .collect::<Vec<_>>();

        let mut map = Map::default();
        let mut pin = map.pin();

        for (key, value) in &keys {
            pin.insert(key, *value);
            assert_eq!(pin.get(key), Some(*value));
        }

        for (key, value) in &keys {
            assert_eq!(pin.get(key), Some(*value));
        }

        drop(pin);

        let mut iter = map.iter_ref();
        let mut count = 0;
        while iter.next().is_some() {
            count += 1;
        }

        assert_eq!(count, keys.len());

        keys.sort_by(|(l, _), (r, _)| l.cmp(r));

        let mut iter = map.iter_ref();
        let mut keys = keys.into_iter();

        while let Some(((lk, lv), (rk, rv))) = iter.next().zip(keys.next()) {
            assert_eq!(*lk, rk);
            assert_eq!(lv, rv);
        }

        map
    }
}
