use core::marker::PhantomData;

use crate::node;
use crate::raw;
use crate::Key;
use crate::Value;

pub struct Map<K: ?Sized, V> {
    pub(crate) raw: raw::concurrent::Map,
    _key: PhantomData<K>,
    _value: PhantomData<V>,
}

impl<K: ?Sized, V> Default for Map<K, V> {
    fn default() -> Self {
        Self {
            raw: raw::concurrent::Map::default(),
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

    pub fn iter_dynamic(&mut self) -> Iter<K, V> {
        Iter {
            inner: self.raw.iter(),
            _key: PhantomData,
            _value: PhantomData,
        }
    }
}

pub struct Iter<'a, K: Key + ?Sized, V> {
    inner: raw::iter::Iter<
        'a,
        K::Stack,
        raw::iter::SelectLeaf,
        raw::iter::Preorder,
        node::SortedIter<'a>,
    >,
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
    pub fn iter_fixed(&mut self) -> impl Iterator<Item = (K, V)> + '_ {
        EntryIter {
            inner: self.raw.iter(),
            _key: PhantomData,
            _value: PhantomData,
        }
    }
}

pub struct EntryIter<'a, K: Key, V> {
    inner: raw::iter::Iter<
        'a,
        K::Stack,
        raw::iter::SelectLeaf,
        raw::iter::Preorder,
        node::SortedIter<'a>,
    >,
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
    raw: raw::concurrent::MapRef<'a>,
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
