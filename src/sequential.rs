use core::cell::Cell;
use core::marker::PhantomData;

use ribbit::atomic::Atomic128;

use crate::iter::postorder;
use crate::iter::PostorderIter;
use crate::iter::PrefixIter;
use crate::iter::Sort;
use crate::stat;
use crate::Edge;
use crate::Key;
use crate::Value;

#[repr(transparent)]
pub struct Map<K, V: Value> {
    root: Atomic128<Edge<V>>,
    _not_sync: PhantomData<Cell<()>>,
    _key: PhantomData<K>,
}

impl<K, V: Value> Default for Map<K, V> {
    fn default() -> Self {
        Self {
            root: Atomic128::default(),
            _not_sync: PhantomData,
            _key: PhantomData,
        }
    }
}

impl<K, V: Value> Map<K, V> {
    pub(crate) fn root(&self) -> &Atomic128<Edge<V>> {
        &self.root
    }

    pub(crate) fn postorder<'g, S: postorder::Selector>(&'g self) -> PostorderIter<'g, V, S> {
        unsafe { PostorderIter::new(&self.root) }
    }
}

impl<K: Key, V: Value> Map<K, V> {
    #[expect(unused_variables)]
    #[inline]
    pub fn get(&self, key: K::Borrow<'_>) -> Option<u64> {
        todo!()
    }

    #[expect(unused_variables)]
    #[inline]
    pub fn insert(&mut self, key: K::Borrow<'_>, value: u64) -> Option<u64> {
        todo!()
    }

    #[expect(unused_variables)]
    #[inline]
    pub fn remove(&mut self, key: K::Borrow<'_>) -> Option<u64> {
        todo!()
    }

    #[expect(unused_variables)]
    #[inline]
    pub fn update(&mut self, key: K::Borrow<'_>, value: u64) -> Option<u64> {
        todo!()
    }

    pub fn iter<S: Sort>(&self) -> Iter<'_, K, V, S> {
        Iter(unsafe { PrefixIter::new(&self.root, K::Write::default()) })
    }
}

pub struct Iter<'g, K: Key, V, S: Sort>(PrefixIter<'g, K::Write, V, S>);

impl<'g, K, V, S> Iter<'g, K, V, S>
where
    K: Key,
    V: Value,
    S: Sort,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(K::Borrow<'_>, V)> {
        self.0
            .lend()
            .map(|(key, value)| (K::Borrow::from(key), unsafe { V::from_u64(value) }))
    }
}

impl<K, V, S> Iterator for Iter<'_, K, V, S>
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

impl<K, V: Value> Drop for Map<K, V> {
    fn drop(&mut self) {
        self.postorder::<V::SelectDrop>().for_each(|edge| unsafe {
            edge.deallocate_unchecked(stat::Counter::FreeDrop);
        })
    }
}
