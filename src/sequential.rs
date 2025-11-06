mod value;
// FIXME: hide from public API
pub mod key;

use core::cell::Cell;
use core::marker::PhantomData;

use ribbit::atomic::Atomic128;

use crate::iter::Sort;
use crate::raw::iter::PostorderIter;
use crate::raw::iter::PrefixIter;
use crate::raw::Edge;
use crate::stat;
pub use key::Key;
pub(crate) use value::Value;

#[repr(transparent)]
pub struct Map<K, V: Value> {
    root: Atomic128<Edge<()>>,
    _not_sync: PhantomData<Cell<()>>,
    _type: PhantomData<(K, V)>,
}

impl<K, V: Value> Default for Map<K, V> {
    fn default() -> Self {
        Self {
            root: Atomic128::default(),
            _not_sync: PhantomData,
            _type: PhantomData,
        }
    }
}

impl<K, V: Value> Map<K, V> {
    pub(crate) fn root(&self) -> &Atomic128<Edge<()>> {
        &self.root
    }

    pub(crate) fn postorder<'g>(&'g self) -> PostorderIter<'g, ()> {
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
        Iter {
            _value: PhantomData,
            iter: unsafe { PrefixIter::new_unchecked(&self.root, K::Read::default()) },
        }
    }
}

pub struct Iter<'g, K: Key, V, S: Sort> {
    _value: PhantomData<V>,
    iter: PrefixIter<'g, 'static, K::Write, (), S>,
}

impl<'g, K, V, S> Iter<'g, K, V, S>
where
    K: Key,
    V: Value,
    S: Sort,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(K::Borrow<'_>, V::Borrow<'g>)> {
        self.iter.lend().map(|(key, value)| {
            (unsafe { K::borrow_writer_unchecked(key) }, unsafe {
                // FIXME: borrow without guard
                V::borrow_from_raw(value)
            })
        })
    }
}

impl<'g, K, V, S> Iterator for Iter<'g, K, V, S>
where
    K: Key,
    V: Value + 'g,
    S: crate::iter::Sort,
{
    type Item = (K, V::Borrow<'g>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.iter.lend().map(|(key, value)| {
            (unsafe { K::from_writer_unchecked(key.clone()) }, unsafe {
                V::borrow_from_raw(value)
            })
        })
    }
}

impl<K, V: Value> Drop for Map<K, V> {
    fn drop(&mut self) {
        self.postorder().for_each(|edge, _| unsafe {
            edge.deallocate(|value| drop(V::from_raw(value)), stat::Counter::FreeDrop);
        })
    }
}
