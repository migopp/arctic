mod entry;
mod iter;
mod value;

pub use entry::Entry;
pub use iter::EntryIter;
pub use iter::EntryIterMut;
pub use iter::Prefix;
pub use iter::PrefixMut;
pub use iter::ValueIter;
pub use iter::ValueIterMut;
pub use value::Value;

use core::cell::Cell;
use core::marker::PhantomData;
use core::ops::RangeFull;

use ribbit::Atomic;

use crate::Ascend;
use crate::Key;
use crate::raw;
use crate::raw::Cursor;
use crate::raw::Edge;
use crate::raw::cursor::path;
use crate::raw::iter::PostorderIter;
use crate::stat;

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
    pub fn get(&self, key: &K::Borrowed) -> Option<&V> {
        let mut cursor = self.cursor(key);
        cursor.traverse_get()?;
        Some(unsafe { cursor.as_value_unchecked().cast::<V>().as_ref() })
    }

    #[inline]
    pub fn get_mut(&mut self, key: &K::Borrowed) -> Option<&mut V> {
        let mut cursor = self.cursor(key);
        cursor.traverse_get()?;
        Some(unsafe { cursor.as_value_unchecked().cast::<V>().as_mut() })
    }

    #[inline]
    pub fn upsert(&mut self, key: &K::Borrowed, value: V) -> Option<V> {
        match self.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(value);
                None
            }
            Entry::Occupied(entry) => Some(entry.insert(value)),
        }
    }

    pub fn entry<'k>(&mut self, key: &'k K::Borrowed) -> Entry<'_, 'k, K, V> {
        let mut cursor = self.cursor(key);

        match cursor.traverse_insert() {
            raw::cursor::Insert::Value {
                old_value: Some(_),
                old: _,
            } => Entry::Occupied(entry::Occupied {
                value: unsafe { cursor.as_value_unchecked().cast::<V>() },
                _value: PhantomData,
            }),
            raw::cursor::Insert::Value {
                old_value: None,
                old: _,
            }
            | raw::cursor::Insert::Replace { .. } => Entry::Vacant(entry::Vacant {
                cursor,
                _value: PhantomData,
            }),
        }
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
        R: raw::iter::Range<K::Read<'k>>,
    {
        let prefix = range.common_prefix();
        Some(unsafe {
            iter::Prefix::new(raw::iter::Prefix::new_range(self.root(), range, prefix)?)
        })
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
        R: raw::iter::Range<K::Read<'k>>,
    {
        Some(unsafe { PrefixMut::new(self.range(range)?) })
    }

    #[inline]
    fn cursor<'k>(&self, key: &'k K::Borrowed) -> Cursor<K::Read<'k>, path::Discard> {
        unsafe { Cursor::<_, path::Discard>::new(self.root(), K::Read::from(key)) }
    }
}

impl<'k, K, V> FromIterator<(&'k K::Borrowed, V)> for Map<K, V>
where
    K: Key,
    V: Value,
{
    fn from_iter<T: IntoIterator<Item = (&'k K::Borrowed, V)>>(iter: T) -> Self {
        let mut map = Map::default();
        for (key, value) in iter {
            map.upsert(key, value);
        }
        map
    }
}

impl<'g, K, V> IntoIterator for &'g Map<K, V>
where
    K: Key,
    V: Value,
{
    type Item = (K, &'g V);
    type IntoIter = EntryIter<'static, 'g, K, V, RangeFull, Ascend>;
    fn into_iter(self) -> Self::IntoIter {
        self.all().entries::<Ascend>()
    }
}

impl<'g, K, V> IntoIterator for &'g mut Map<K, V>
where
    K: Key,
    V: Value,
{
    type Item = (K, &'g mut V);
    type IntoIter = EntryIterMut<'static, 'g, K, V, RangeFull, Ascend>;
    fn into_iter(self) -> Self::IntoIter {
        self.all_mut().entries_mut::<Ascend>()
    }
}

impl<K, V> Drop for Map<K, V>
where
    K: Key,
    V: Value,
{
    fn drop(&mut self) {
        self.postorder().for_each_internal(|edge, _| unsafe {
            edge.deallocate(|value| drop(V::from_raw(value)), stat::Counter::FreeDrop);
        })
    }
}
