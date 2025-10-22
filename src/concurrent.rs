use core::marker::PhantomData;

use crate::raw;
use crate::sequential;
use crate::Key;
use crate::Value;

pub struct Map<K, V> {
    raw: raw::concurrent::Map<V>,
    _key: PhantomData<K>,
}

impl<K, V> Default for Map<K, V> {
    fn default() -> Self {
        Self {
            raw: raw::concurrent::Map::<V>::default(),
            _key: PhantomData,
        }
    }
}

impl<K, V> Map<K, V> {
    #[inline]
    pub fn as_sequential(&mut self) -> &mut sequential::Map<K, V> {
        unsafe {
            core::mem::transmute::<&mut raw::sequential::Map<V>, &mut sequential::Map<K, V>>(
                self.raw.as_sequential(),
            )
        }
    }

    #[inline]
    pub fn pin(&self) -> MapRef<K, V> {
        MapRef {
            raw: self.raw.pin(),
            _key: PhantomData,
        }
    }
}

pub struct MapRef<'g, K, V> {
    raw: raw::concurrent::MapRef<'g, V>,
    _key: PhantomData<K>,
}

impl<'g, K, V> MapRef<'g, K, V>
where
    K: Key,
    V: Value + Send + Sync,
{
    pub fn get<'l, 'k>(&'l mut self, key: K::Borrow<'k>) -> Option<V::Shared<'g, 'l>> {
        self.raw.get(K::Read::from(key))
    }

    pub fn insert<'l, 'k>(&'l mut self, key: K::Borrow<'k>, value: V) -> Option<V::Owned<'g, 'l>> {
        self.raw.insert(K::Read::from(key), value)
    }

    pub fn remove<'k>(&mut self, key: K::Borrow<'k>) -> Option<V> {
        self.raw.remove(K::Read::from(key)).map(V::from_u64)
    }

    pub fn update<'k>(&mut self, key: K::Borrow<'k>, value: V) -> Option<V> {
        self.raw.update(K::Read::from(key), value).map(V::from_u64)
    }

    pub fn prefix_non_linearizable<'l, S: crate::iter::Sort>(
        &'l mut self,
        prefix: impl Into<K::Read<'l>>,
    ) -> PrefixNonLinearizable<'g, 'l, K, V, S> {
        PrefixNonLinearizable {
            iter: self.raw.prefix_non_linearizable(prefix.into()),
            _value: PhantomData,
        }
    }

    pub fn range_non_linearizable<'l>(
        &'l mut self,
        min: impl Into<K::Read<'l>>,
        max: impl Into<K::Read<'l>>,
    ) -> RangeIter<'g, 'l, K, V> {
        RangeIter {
            iter: self
                .raw
                .range_non_linearizable::<_, _>(min.into(), max.into()),
            _value: PhantomData,
        }
    }

    pub fn range_pessimistic<'l>(
        &'l mut self,
        min: impl Into<K::Read<'l>>,
        max: impl Into<K::Read<'l>>,
        output: &mut Vec<(K, V)>,
    ) {
        self.raw
            .range_pessimistic::<_>(min.into(), max.into(), output);
    }

    pub fn range_optimistic<'l>(
        &'l mut self,
        min: impl Into<K::Read<'l>>,
        max: impl Into<K::Read<'l>>,
        retry: usize,
        output: &mut Vec<(K, V)>,
    ) {
        self.raw
            .range_optimistic::<K>(min.into(), max.into(), retry, output)
    }
}

pub struct PrefixNonLinearizable<'g, 'l, K: Key, V, S: crate::iter::Sort> {
    iter: raw::concurrent::PrefixNonLinearizable<'g, 'l, K::Write, V, S>,
    _value: PhantomData<V>,
}

impl<'g, 'l, K: Key, V: Value, S: crate::iter::Sort> PrefixNonLinearizable<'g, 'l, K, V, S> {
    #[inline]
    pub fn lend<'k>(&'k mut self) -> Option<(K::Borrow<'k>, V)> {
        self.iter
            .lend()
            .map(|(key, value)| (K::Borrow::from(key), V::from_u64(value)))
    }
}

impl<'g, 'l, K, V, S> Iterator for PrefixNonLinearizable<'g, 'l, K, V, S>
where
    K: Key,
    V: Value,
    S: crate::iter::Sort,
{
    type Item = (K, V);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.lend().map(|(key, value)| (K::from(key), value))
    }
}

pub struct RangeIter<'g, 'l, K: Key, V> {
    iter: raw::concurrent::RangeIter<'g, 'l, K::Read<'l>, K::Write, V>,
    _value: PhantomData<V>,
}

impl<'g, 'l, K: Key, V: Value> RangeIter<'g, 'l, K, V> {
    #[inline]
    pub fn lend<'k>(&'k mut self) -> Option<(K::Borrow<'k>, V)> {
        self.iter
            .lend()
            .map(|(key, value)| (K::Borrow::from(key), V::from_u64(value)))
    }

    #[inline]
    pub fn for_each<F: FnMut(K::Borrow<'_>, V)>(mut self, mut apply: F) {
        self.iter
            .for_each(|key, value| apply(K::Borrow::from(key), V::from_u64(value)))
    }
}

impl<'g, 'l, K, V> Iterator for RangeIter<'g, 'l, K, V>
where
    K: Key,
    V: Value,
{
    type Item = (K, V);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.lend().map(|(key, value)| (K::from(key), value))
    }
}
