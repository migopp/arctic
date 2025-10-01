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

pub struct MapRef<'g, K: ?Sized, V> {
    raw: raw::concurrent::MapRef<'g>,
    _key: PhantomData<K>,
    _value: PhantomData<V>,
}

impl<'g, K: Key + ?Sized, V: Value> MapRef<'g, K, V> {
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

    pub fn range_non_linearizable<'l, R>(&'l mut self, range: R) -> RangeIter<'g, 'l, K, V>
    where
        R: RangeBounds<&'l K>,
    {
        let start = range.start_bound().map(|start| start.read());
        let end = range.end_bound().map(|end| end.read());
        RangeIter {
            iter: self.raw.range_non_linearizable(start, end),
            _value: PhantomData,
        }
    }
}

pub struct RangeIter<'g, 'l, K: Key + ?Sized + 'l, V> {
    iter: raw::concurrent::RangeIter<'g, 'l, K::Read<'l>, K::Write>,
    _value: PhantomData<V>,
}

impl<'g, 'l, K: Key + ?Sized + 'l, V: Value> RangeIter<'g, 'l, K, V> {
    #[inline]
    pub fn lend(&mut self) -> Option<(&K::Write, V)> {
        self.iter
            .lend()
            .map(|(key, value)| (key, V::from_u64(value)))
    }
}

impl<'g, 'l, K, V> Iterator for RangeIter<'g, 'l, K, V>
where
    K: Key + for<'b> From<&'b K::Write>,
    V: Value,
{
    type Item = (K, V);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.lend().map(|(key, value)| (K::from(key), value))
    }
}
