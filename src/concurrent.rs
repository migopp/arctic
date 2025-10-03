use core::marker::PhantomData;
use core::ops::RangeBounds;

use crate::raw;
use crate::sequential;
use crate::Key;
use crate::Value;

pub struct Map<K, V> {
    raw: raw::concurrent::Map,
    _key: PhantomData<K>,
    _value: PhantomData<V>,
}

impl<K, V> Default for Map<K, V> {
    fn default() -> Self {
        Self {
            raw: raw::concurrent::Map::default(),
            _key: PhantomData,
            _value: PhantomData,
        }
    }
}

impl<K, V> Map<K, V> {
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

pub struct MapRef<'g, K, V> {
    raw: raw::concurrent::MapRef<'g>,
    _key: PhantomData<K>,
    _value: PhantomData<V>,
}

impl<'g, K: Key, V: Value> MapRef<'g, K, V> {
    pub fn get<'k>(&self, key: K::Borrow<'k>) -> Option<V> {
        self.raw.get(K::Read::from(key)).map(V::from_u64)
    }

    pub fn insert<'k>(&mut self, key: K::Borrow<'k>, value: V) -> Option<V> {
        self.raw
            .insert(K::Read::from(key), value.into_u64())
            .map(V::from_u64)
    }

    pub fn remove<'k>(&mut self, key: K::Borrow<'k>) -> Option<V> {
        self.raw.remove(K::Read::from(key)).map(V::from_u64)
    }

    pub fn update<'k>(&mut self, key: K::Borrow<'k>, value: V) -> Option<V> {
        self.raw
            .update(K::Read::from(key), value.into_u64())
            .map(V::from_u64)
    }

    pub fn range_non_linearizable<'l, B>(
        &'l mut self,
        range: B,
    ) -> RangeNonLinearizableIter<'g, 'l, B, K, V>
    where
        B: RangeBounds<K::Borrow<'l>> + Clone,
    {
        RangeNonLinearizableIter {
            iter: self.raw.range_non_linearizable(range),
            _value: PhantomData,
        }
    }

    // pub fn range<'l, R>(&'l mut self, range: R) -> impl Iterator<Item = (K, V)>
    // where
    //     R: RangeBounds<K::Borrow<'l>> + Clone,
    // {
    //     self.raw
    //         .range(range)
    //         .map(|(key, value)| (K::from_owned(key), V::from_u64(value)))
    // }
}

pub struct RangeNonLinearizableIter<'g, 'l, B, K: Key + 'l, V> {
    iter: raw::concurrent::RangeNonLinearizableIter<'g, 'l, B, K::Borrow<'l>, K::Write>,
    _value: PhantomData<V>,
}

impl<'g, 'l, R: RangeBounds<K::Borrow<'l>>, K: Key + 'l, V: Value>
    RangeNonLinearizableIter<'g, 'l, R, K, V>
{
    #[inline]
    pub fn lend<'k>(&'k mut self) -> Option<(K::Borrow<'k>, V)> {
        self.iter
            .lend()
            .map(|(key, value)| (K::from_borrowed(key), V::from_u64(value)))
    }
}

impl<'g, 'l, R, K, V> Iterator for RangeNonLinearizableIter<'g, 'l, R, K, V>
where
    R: RangeBounds<K::Borrow<'l>>,
    K: Key,
    V: Value,
{
    type Item = (K, V);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.iter
            .lend()
            .map(|(key, value)| (K::from_owned(key.clone()), V::from_u64(value)))
    }
}
