use core::marker::PhantomData;
use core::ops::RangeBounds;

use crate::raw;
use crate::sequential;
use crate::Key;
use crate::Value;

pub struct Map<K: ?Sized, V> {
    raw: raw::concurrent::Map,
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

impl<K: ?Sized, V> Map<K, V> {
    #[inline]
    pub fn as_sequential(&mut self) -> &mut sequential::Map<K, V> {
        unsafe {
            core::mem::transmute::<&mut raw::sequential::Map, &mut sequential::Map<K, V>>(
                self.raw.as_sequential(),
            )
        }
    }

    #[inline]
    pub fn pin(&self) -> MapRef<K, V> {
        MapRef {
            raw: self.raw.pin(),
            _key: PhantomData,
            _value: PhantomData,
        }
    }
}

pub struct MapRef<'a, K: ?Sized, V> {
    raw: raw::concurrent::MapRef<'a>,
    _key: PhantomData<K>,
    _value: PhantomData<V>,
}

impl<K: Key + ?Sized, V: Value> MapRef<'_, K, V> {
    pub fn get(&self, key: &K) -> Option<V> {
        self.raw.get(key.read()).map(V::from_u64)
    }

    pub fn insert(&mut self, key: &K, value: V) -> Option<V> {
        self.raw
            .insert(key.read(), value.into_u64())
            .map(V::from_u64)
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        self.raw.remove(key.read()).map(V::from_u64)
    }

    pub fn update(&mut self, key: &K, value: V) -> Option<V> {
        self.raw
            .update(key.read(), value.into_u64())
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

impl<'a, K: Key + ?Sized, V: Value> MapRef<'a, K, V> {
    pub fn range_non_linearizable<'k, R: RangeBounds<&'k K>>(
        &mut self,
        range: R,
    ) -> RangeIter<'a, 'k, K, impl RangeBounds<K::Read<'k>>, V> {
        let start = range.start_bound().map(|start| start.read());
        let end = range.end_bound().map(|end| end.read());
        RangeIter {
            iter: self.raw.range_non_linearizable((start, end)),
            _value: PhantomData,
        }
    }
}

pub struct RangeIter<'a, 'k, K: Key + ?Sized + 'k, R: RangeBounds<K::Read<'k>>, V> {
    iter: raw::concurrent::RangeIter<'a, R, K::Read<'k>, K::Write>,
    _value: PhantomData<V>,
}

impl<'a, 'k, K: Key + ?Sized + 'k, R: RangeBounds<K::Read<'k>>, V: Value>
    RangeIter<'a, 'k, K, R, V>
{
    #[inline]
    pub fn lend(&mut self) -> Option<(&K::Write, V)> {
        self.iter
            .lend()
            .map(|(key, value)| (key, V::from_u64(value)))
    }
}

impl<'a, 'k, K, R, V> Iterator for RangeIter<'a, 'k, K, R, V>
where
    R: RangeBounds<K::Read<'k>>,
    K: Key + for<'b> From<&'b K::Write>,
    V: Value,
{
    type Item = (K, V);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.lend().map(|(key, value)| (K::from(key), value))
    }
}
