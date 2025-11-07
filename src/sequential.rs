mod value;
// FIXME: hide from public API
pub mod key;

use core::cell::Cell;
use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use key::Read as _;
use ribbit::atomic::Atomic128;

use crate::byte;
use crate::iter::Order;
use crate::raw;
use crate::raw::edge;
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
        let mut edge = self.root();
        let mut key = K::Read::from(key);

        loop {
            let old = edge.load_packed(Ordering::Relaxed);
            let old_meta = old.meta();
            let save = key;
            let r#match = old_meta.key().match_split(&mut key);

            // Fast path: traverse
            if let byte::MatchSplit::Full(len) = r#match {
                if let Some(node) = old.as_node() {
                    let byte = key.next().unwrap();
                    let node = unsafe { node.into_ref_unchecked() };
                    if let Some(next) = node.get_or_reserve(byte) {
                        edge = next;
                        continue;
                    }
                }
            }

            let new = match r#match {
                byte::MatchSplit::Full(_) => match old.child() {
                    Some(edge::Child::Node(node)) => {
                        // node.expand([(key.next(), Self::insert_help(key, value))]);
                        todo!()
                    }
                    None | Some(edge::Child::Value(_)) => Self::insert_help(key, value),
                },
                byte::MatchSplit::Partial { start, middle, end } => {
                    key.take(start.len());
                    let byte = key.next().unwrap();
                    Edge::new_node::<raw::node::Node3<()>, _>(
                        start,
                        [
                            (byte, Self::insert_help(key, value)),
                            (middle, old.with_meta(old.meta().with_key(end))),
                        ],
                    )
                }
            };

            edge.store_packed(new, Ordering::Relaxed);
            return old.as_value();
        }
    }

    fn insert_help(mut key: K::Read<'_>, value: u64) -> ribbit::Packed<Edge<()>> {
        if key.bits() > byte::Len::MAX.bits() as usize {
            let prefix = key.take(byte::Len::MAX);
            let byte = key.next().unwrap();
            Edge::new_node::<raw::node::Node3<()>, _>(
                prefix,
                [(byte, Self::insert_help(key, value))],
            )
        } else {
            let prefix = key.take(unsafe { byte::Len::from_bits_unchecked(key.bits() as u8) });
            Edge::new_value(prefix, value)
        }
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

    pub fn iter<O: Order>(&self) -> Iter<'_, K, V, O> {
        Iter {
            _value: PhantomData,
            iter: unsafe { PrefixIter::new_unchecked(&self.root, K::Read::default()) },
        }
    }
}

pub struct Iter<'g, K: Key, V, O: Order> {
    _value: PhantomData<V>,
    iter: PrefixIter<'g, K::Write, (), O>,
}

impl<'g, K, V, O> Iter<'g, K, V, O>
where
    K: Key,
    V: Value,
    O: Order,
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

impl<K, V: Value> Drop for Map<K, V> {
    fn drop(&mut self) {
        self.postorder().for_each(|edge, _| unsafe {
            edge.deallocate(|value| drop(V::from_raw(value)), stat::Counter::FreeDrop);
        })
    }
}
