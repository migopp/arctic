use core::marker::PhantomData;
use core::ops::RangeBounds;
use core::ops::RangeFull;

use crate::node;
use crate::raw;
use crate::Key;
use crate::Value;

#[repr(transparent)]
pub struct Map<K, V> {
    raw: raw::sequential::Map,
    _key: PhantomData<K>,
    _value: PhantomData<V>,
}

impl<K, V> Default for Map<K, V> {
    fn default() -> Self {
        Self {
            raw: raw::sequential::Map::default(),
            _key: PhantomData,
            _value: PhantomData,
        }
    }
}

impl<K, V> Map<K, V> {
    pub(crate) fn as_raw(&mut self) -> &mut raw::sequential::Map {
        &mut self.raw
    }
}

impl<K: Key, V: Value> Map<K, V> {
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

    #[expect(private_interfaces)]
    pub fn iter(&self) -> Iter<RangeFull, K, V, node::SortedIter> {
        Iter {
            inner: self.raw.iter_leaves(RangeFull),
            _key: PhantomData,
            _value: PhantomData,
        }
    }

    #[expect(private_interfaces)]
    pub fn iter_unsorted(&self) -> Iter<RangeFull, K, V, node::UnsortedIter> {
        Iter {
            inner: self.raw.iter_leaves(RangeFull),
            _key: PhantomData,
            _value: PhantomData,
        }
    }
}

pub(crate) struct Iter<'a, 'k, R, K: Key + 'k, V, S: raw::iter::Sort<'a>> {
    inner: raw::iter::LeafIter<'a, R, K::Borrow<'k>, K::Write, S>,
    _key: PhantomData<K>,
    _value: PhantomData<V>,
}

impl<'a, 'k, R, K, V, S> Iterator for Iter<'a, 'k, R, K, V, S>
where
    R: RangeBounds<K::Borrow<'k>>,
    K: Key + 'k,
    V: Value,
    S: raw::iter::Sort<'a>,
{
    type Item = (K, V);
    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .lend()
            .map(|(key, value)| (K::from_owned(key.clone()), V::from_u64(value)))
    }
}

impl<'a, 'k, R, K, V, S> Iter<'a, 'k, R, K, V, S>
where
    R: RangeBounds<K::Borrow<'k>>,
    K: Key + 'k,
    V: Value,
    S: raw::iter::Sort<'a>,
{
    #[allow(dead_code)]
    pub fn lend<'i>(&'i mut self) -> Option<(K::Borrow<'i>, V)> {
        self.inner
            .lend()
            .map(|(key, value)| (K::from_borrowed(key), V::from_u64(value)))
    }
}
