mod value;

use core::cell::Cell;
use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use ribbit::Atomic;

use crate::iter::Order;
use crate::raw;
use crate::raw::edge;
use crate::raw::edge::Key as _;
use crate::raw::edge::Meta as _;
use crate::raw::iter::PostorderIter;
use crate::raw::iter::RangeIter;
use crate::raw::key::Read as _;
use crate::raw::Edge;
use crate::stat;
use crate::Key;
pub(crate) use value::Value;

#[repr(transparent)]
pub struct Map<K: Key, V: Value> {
    root: Atomic<Edge<K::Edge>>,
    _not_sync: PhantomData<Cell<()>>,
    _value: PhantomData<V>,
}

impl<K: Key, V: Value> Default for Map<K, V> {
    fn default() -> Self {
        Self {
            root: Atomic::new_packed(Edge::DEFAULT),
            _not_sync: PhantomData,
            _value: PhantomData,
        }
    }
}

impl<K: Key, V: Value> Map<K, V> {
    pub(crate) fn root(&self) -> &Atomic<Edge<K::Edge>> {
        &self.root
    }

    pub(crate) fn postorder<'g>(&'g self) -> PostorderIter<'g, K::Edge> {
        unsafe { PostorderIter::new(&self.root) }
    }

    #[inline]
    pub fn get(&self, key: <K as Key>::Borrow<'_>) -> Option<V::Borrow<'_>> {
        unsafe { raw::cursor::Point::<K, _>::get(&self.root, K::Read::from(key)) }
            .map(|value| unsafe { V::borrow_from_raw(value) })
    }

    #[expect(unused_variables)]
    #[inline]
    pub fn insert(&mut self, key: <K as Key>::Borrow<'_>, value: V) -> Option<V> {
        let mut edge = self.root();
        let mut reader = K::Read::from(key);

        loop {
            let old = edge.load_packed(Ordering::Relaxed);
            let old_key = old.meta().key();
            let old_len = old_key.len();

            let key = reader.read(old_len);

            // Fast path: traverse
            if key == old_key {
                if let Some(node) = old.as_node() {
                    let byte = reader.next().unwrap();
                    let node = unsafe { node.into_ref_unchecked() };
                    if let Some(next) = node.get_or_reserve(byte) {
                        edge = next;
                        continue;
                    }
                }
            }

            let new = match old.meta().expand(key) {
                Err(()) => match old.child() {
                    Some(edge::Child::Node(node)) => {
                        // node.expand([(key.next(), Self::insert_help(key, value))]);
                        todo!()
                    }
                    None | Some(edge::Child::Value(_)) => Self::insert_help(reader, value),
                },
                Ok((start, middle, end)) => {
                    let byte = reader.next().unwrap();
                    Edge::new_node::<raw::node::Node3<_>, _>(
                        start,
                        [
                            (byte, Self::insert_help(reader, value)),
                            (middle, old.with_meta(end)),
                        ],
                    )
                }
            };

            edge.store_packed(new, Ordering::Relaxed);
            return old.as_value().map(|value| unsafe { V::from_raw(value) });
        }
    }

    fn insert_help(mut reader: K::Read<'_>, value: V) -> ribbit::Packed<Edge<K::Edge>> {
        let prefix = reader.read(<ribbit::Packed<K::Edge> as edge::Meta>::MAX_LEN);

        if reader.bits() > 0 {
            let byte = reader.next().unwrap();
            Edge::new_node::<raw::node::Node3<_>, _>(
                prefix,
                [(byte, Self::insert_help(reader, value))],
            )
        } else {
            Edge::new_value(prefix, value.into_raw())
        }
    }

    #[expect(unused_variables)]
    #[inline]
    pub fn remove(&mut self, key: <K as Key>::Borrow<'_>) -> Option<V> {
        todo!()
    }

    #[expect(unused_variables)]
    #[inline]
    pub fn update(&mut self, key: <K as Key>::Borrow<'_>, value: V) -> Result<Option<V>, V> {
        todo!()
    }

    pub fn iter<O: Order>(&self) -> Iter<'_, K, V, O> {
        Iter {
            _value: PhantomData,
            iter: unsafe { RangeIter::new_unchecked(&self.root, K::Read::default(), ..) },
        }
    }
}

pub struct Iter<'g, K: Key, V, O: Order> {
    _value: PhantomData<V>,
    iter: RangeIter<'g, K::Read<'g>, K::Write, K::Edge, core::ops::RangeFull, O>,
}

impl<'g, K, V, O> Iter<'g, K, V, O>
where
    K: Key,
    V: Value,
    O: Order,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(<K as Key>::Borrow<'_>, V::Borrow<'g>)> {
        self.iter.lend().map(|(key, value)| {
            (unsafe { K::borrow_writer_unchecked(key) }, unsafe {
                // FIXME: borrow without guard
                V::borrow_from_raw(value)
            })
        })
    }
}

impl<'g, K, V, O> Iterator for Iter<'g, K, V, O>
where
    K: Key,
    V: Value + 'g,
    O: crate::iter::Order,
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

impl<K: Key, V: Value> Drop for Map<K, V> {
    fn drop(&mut self) {
        self.postorder().for_each(|edge, _| unsafe {
            edge.deallocate(|value| drop(V::from_raw(value)), stat::Counter::FreeDrop);
        })
    }
}
