use core::marker::PhantomData;

use crate::node;
use crate::raw;
use crate::Key;
use crate::Value;

#[repr(transparent)]
pub struct Map<K: ?Sized, V> {
    raw: raw::sequential::Map,
    _key: PhantomData<K>,
    _value: PhantomData<V>,
}

impl<K: ?Sized, V> Default for Map<K, V> {
    fn default() -> Self {
        Self {
            raw: raw::sequential::Map::default(),
            _key: PhantomData,
            _value: PhantomData,
        }
    }
}

impl<K: ?Sized, V> Map<K, V> {
    pub(crate) fn as_raw(&mut self) -> &mut raw::sequential::Map {
        &mut self.raw
    }
}

impl<K: ?Sized + Key, V: Value> Map<K, V> {
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

    #[expect(private_interfaces)]
    pub fn iter_dynamic(&self) -> DynamicIter<K, V, node::SortedIter> {
        DynamicIter {
            inner: self.raw.iter(raw::iter::SelectLeaf),
            _key: PhantomData,
            _value: PhantomData,
        }
    }

    #[expect(private_interfaces)]
    pub fn iter_unsorted_dynamic(&self) -> DynamicIter<K, V, node::UnsortedIter> {
        DynamicIter {
            inner: self.raw.iter(raw::iter::SelectLeaf),
            _key: PhantomData,
            _value: PhantomData,
        }
    }
}

impl<K, V> Map<K, V>
where
    K: Key + for<'s> From<&'s K::Stack>,
    V: Value,
{
    pub fn iter_fixed(&self) -> impl Iterator<Item = (K, V)> + '_ {
        FixedIter::<K, V, node::SortedIter> {
            inner: self.raw.iter(raw::iter::SelectLeaf),
            _key: PhantomData,
            _value: PhantomData,
        }
    }

    pub fn iter_unsorted_fixed(&self) -> impl Iterator<Item = (K, V)> + '_ {
        FixedIter::<K, V, node::UnsortedIter> {
            inner: self.raw.iter(raw::iter::SelectLeaf),
            _key: PhantomData,
            _value: PhantomData,
        }
    }
}

pub(crate) struct FixedIter<'a, K: Key + ?Sized, V, S: raw::iter::Sort<'a>> {
    inner: raw::iter::Iter<'a, K::Stack, raw::iter::SelectLeaf, raw::iter::Preorder, S>,
    _key: PhantomData<K>,
    _value: PhantomData<V>,
}

impl<'a, K, V, S> Iterator for FixedIter<'a, K, V, S>
where
    K: Key,
    K: for<'s> From<&'s K::Stack>,
    V: Value,
    S: raw::iter::Sort<'a>,
{
    type Item = (K, V);
    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|(key, value)| (K::from(key), V::from_u64(value)))
    }
}

#[expect(private_bounds)]
pub struct DynamicIter<'a, K: Key + ?Sized, V, S: raw::iter::Sort<'a>> {
    inner: raw::iter::Iter<'a, K::Stack, raw::iter::SelectLeaf, raw::iter::Preorder, S>,
    _key: PhantomData<K>,
    _value: PhantomData<V>,
}

#[expect(private_bounds)]
impl<'a, K: Key + ?Sized, V: Value, S: raw::iter::Sort<'a>> DynamicIter<'a, K, V, S> {
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<(&K::Stack, V)> {
        self.inner
            .next()
            .map(|(key, value)| (key, V::from_u64(value)))
    }
}
