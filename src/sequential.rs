mod value;

use core::cell::Cell;
use core::marker::PhantomData;

use ribbit::Atomic;

use crate::raw::cursor::path;
use crate::raw::cursor::CursorMut;
use crate::raw::iter::PostorderIter;
use crate::raw::iter::RangeIter;
use crate::raw::Cursor;
use crate::raw::Edge;
use crate::raw::Frozen;
use crate::stat;
use crate::Key;
pub use value::Value;

#[repr(transparent)]
pub struct Map<K: Key, V: Value> {
    root: Atomic<Edge<K::Edge>>,
    _not_sync: PhantomData<Cell<()>>,
    _value: PhantomData<V>,
}

impl<K, V> Default for Map<K, V>
where
    K: Key,
    V: Value,
{
    fn default() -> Self {
        Self {
            root: Atomic::new_packed(Edge::DEFAULT),
            _not_sync: PhantomData,
            _value: PhantomData,
        }
    }
}

impl<K, V> Map<K, V>
where
    K: Key,
    V: Value,
{
    pub(crate) fn root(&self) -> &Atomic<Edge<K::Edge>> {
        &self.root
    }

    pub(crate) fn postorder<'g>(&'g self) -> PostorderIter<'g, K::Edge> {
        unsafe { PostorderIter::new(&self.root) }
    }

    #[inline]
    pub fn get(&self, key: K::Borrow<'_>) -> Option<V::Borrow<'_>> {
        let reader = K::Read::from(key);
        let value =
            unsafe { Cursor::<K, path::Discard>::new(self.root(), reader) }.traverse_get()?;
        Some(unsafe { V::borrow_from_raw(value) })
    }

    #[inline]
    pub fn get_mut(&mut self, key: K::Borrow<'_>) -> Option<V::BorrowMut<'_>> {
        let reader = K::Read::from(key);
        let value =
            unsafe { Cursor::<K, path::Discard>::new(self.root(), reader) }.traverse_get()?;
        Some(unsafe { V::borrow_mut_from_raw(value) })
    }

    #[inline]
    pub fn upsert(&mut self, key: K::Borrow<'_>, value: V) -> Option<V> {
        let reader = K::Read::from(key);
        let mut cursor = CursorMut::<K>::new(&mut self.root, reader);
        let new_value = V::into_raw(value);

        loop {
            match cursor.traverse_insert() {
                crate::raw::cursor::Insert::Value {
                    old_value,
                    old,
                    key,
                } => match cursor.insert(old, key, new_value) {
                    Err(Frozen) => unreachable!(),
                    Ok(new) => {
                        cursor.edge_mut().set_packed(new);
                        return old_value.map(|old| unsafe { V::from_raw(old) });
                    }
                },
                crate::raw::cursor::Insert::Smo(Ok((_, old, new))) => {
                    cursor.edge_mut().set_packed(new);
                    if let Some(node) = old.as_node() {
                        unsafe { node.deallocate(stat::Counter::FreeRetire) };
                    }
                }
                crate::raw::cursor::Insert::Smo(Err(Frozen)) => unreachable!(),
            }
        }
    }

    #[expect(unused_variables)]
    #[inline]
    pub fn remove(&mut self, key: K::Borrow<'_>) -> Option<V> {
        todo!()
    }

    #[expect(unused_variables)]
    #[inline]
    pub fn update(&mut self, key: K::Borrow<'_>, value: V) -> Result<Option<V>, V> {
        todo!()
    }

    pub fn iter<const REVERSE: bool>(&self) -> Iter<'static, '_, REVERSE, K, V> {
        Iter {
            _value: PhantomData,
            iter: unsafe { RangeIter::new_unchecked(&self.root, K::Read::default(), ..) },
        }
    }
}

pub struct Iter<'k, 'g, const REVERSE: bool, K: Key, V: Value> {
    _value: PhantomData<&'g V>,
    iter: RangeIter<'k, 'g, REVERSE, K, core::ops::RangeFull, K::Write>,
}

impl<'k, 'g, const REVERSE: bool, K, V> Iter<'k, 'g, REVERSE, K, V>
where
    K: Key,
    V: Value,
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

impl<'k, 'g, const REVERSE: bool, K, V> Iterator for Iter<'k, 'g, REVERSE, K, V>
where
    K: Key,
    V: Value,
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

impl<K, V> Drop for Map<K, V>
where
    K: Key,
    V: Value,
{
    fn drop(&mut self) {
        self.postorder().for_each(|edge, _| unsafe {
            edge.deallocate(|value| drop(V::from_raw(value)), stat::Counter::FreeDrop);
        })
    }
}
