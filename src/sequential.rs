mod value;

use core::cell::Cell;
use core::marker::PhantomData;

use ribbit::Atomic;

use crate::raw::cursor::path;
use crate::raw::iter::PostorderIter;
use crate::raw::iter::RangeIter;
use crate::raw::Cursor;
use crate::raw::Edge;
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
    pub fn insert(&mut self, _key: K::Borrow<'_>, _value: V) -> Option<V> {
        todo!()
        // let mut edge = self.root();
        // let mut reader = K::Read::from(key);
        //
        // loop {
        //     let old = edge.load_packed(Ordering::Relaxed);
        //     let old_key = old.meta().key();
        //     let old_len = old_key.len();
        //
        //     let key = reader.read(old_len);
        //
        //     // Fast path: traverse
        //     if key == old_key {
        //         if let Some(node) = old.as_node() {
        //             let byte = reader.next().unwrap();
        //             let node = unsafe { node.into_ref_unchecked() };
        //             if let Some(next) = node.get_or_insert(byte) {
        //                 edge = next;
        //                 continue;
        //             }
        //         }
        //     }
        //
        //     let new = match old.meta().expand(key) {
        //         Err(()) => match old.child() {
        //             Some(edge::Child::Node(node)) => {
        //                 // node.expand([(key.next(), Self::insert_help(key, value))]);
        //                 todo!()
        //             }
        //             None | Some(edge::Child::Value(_)) => Self::insert_help(reader, value),
        //         },
        //         Ok((start, middle, end)) => {
        //             let byte = reader.next().unwrap();
        //             Edge::new_node::<raw::node::Node3<_>, _>(
        //                 start,
        //                 [
        //                     (byte, Self::insert_help(reader, value)),
        //                     (middle, old.with_meta(end)),
        //                 ],
        //             )
        //         }
        //     };
        //
        //     edge.store_packed(new, Ordering::Relaxed);
        //     return old.as_value().map(|value| unsafe { V::from_raw(value) });
        // }
    }

    #[expect(unused)]
    fn insert_help(mut _reader: K::Read<'_>, _value: V) -> ribbit::Packed<Edge<K::Edge>> {
        todo!()
        // let prefix = reader.read(<ribbit::Packed<K::Edge> as edge::Meta>::MAX_LEN);
        //
        // if reader.bits() > 0 {
        //     let byte = reader.next().unwrap();
        //     Edge::new_node::<raw::node::Node3<_>, _>(
        //         prefix,
        //         [(byte, Self::insert_help(reader, value))],
        //     )
        // } else {
        //     Edge::new_value(prefix, value.into_raw())
        // }
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
