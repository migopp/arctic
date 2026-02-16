mod iter;
mod value;

pub use iter::EntryIter;
pub use iter::Prefix;
pub use iter::PrefixMut;
pub use iter::ValueIter;
pub use value::Value;

use core::cell::Cell;
use core::marker::PhantomData;
use core::ops::RangeFull;

use ribbit::Atomic;

use crate::raw;
use crate::raw::cursor::path;
use crate::raw::cursor::CursorMut;
use crate::raw::iter::PostorderIter;
use crate::raw::Cursor;
use crate::raw::Edge;
use crate::raw::Frozen;
use crate::stat;
use crate::Key;

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

    pub fn all(&self) -> Prefix<'static, '_, K, V, RangeFull> {
        unsafe { Prefix::new(raw::iter::Prefix::<K>::new_all(self.root())) }
    }

    pub fn prefix<'k>(
        &self,
        prefix: impl Into<K::Read<'k>>,
    ) -> Option<Prefix<'k, '_, K, V, RangeFull>> {
        let prefix = prefix.into();
        let prefix = unsafe { raw::iter::Prefix::<K>::new_prefix(self.root(), prefix) }?;
        Some(unsafe { Prefix::new(prefix) })
    }

    pub fn range<'k, R>(&self, range: R) -> Option<Prefix<'k, '_, K, V, R>>
    where
        R: raw::iter::Range<'k, K>,
    {
        let prefix = unsafe { raw::iter::Prefix::new_range(self.root(), range) }?;
        Some(unsafe { iter::Prefix::new(prefix) })
    }

    pub fn all_mut(&mut self) -> PrefixMut<'static, '_, K, V, RangeFull> {
        unsafe { PrefixMut::new(self.all()) }
    }

    pub fn prefix_mut<'k>(
        &mut self,
        prefix: impl Into<K::Read<'k>>,
    ) -> Option<PrefixMut<'k, '_, K, V, RangeFull>> {
        Some(unsafe { PrefixMut::new(self.prefix(prefix)?) })
    }

    pub fn range_mut<'k, R>(&mut self, range: R) -> Option<PrefixMut<'k, '_, K, V, R>>
    where
        R: raw::iter::Range<'k, K>,
    {
        Some(unsafe { PrefixMut::new(self.range(range)?) })
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
