mod iter;
mod value;

pub use iter::EntryIter;
pub use iter::EntryIterMut;
pub use iter::Prefix;
pub use iter::PrefixMut;
pub use iter::ValueIter;
pub use iter::ValueIterMut;
pub use value::Value;

use core::cell::Cell;
use core::marker::PhantomData;
use core::ops::ControlFlow;
use core::ops::RangeFull;

use ribbit::Atomic;

use crate::Ascend;
use crate::Key;
use crate::raw;
use crate::raw::Cursor;
use crate::raw::Edge;
use crate::raw::Frozen;
use crate::raw::cursor::CursorMut;
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

pub enum Update<'g, V, B>
where
    V: Value + 'g,
{
    Absent { value: Option<V> },
    Success { old: V },
    Break { old: V::Borrow<'g>, r#break: B },
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

    #[inline]
    pub fn remove(&mut self, key: K::Borrow<'_>) -> Option<V> {
        match self.update_with_impl(key, |_| ControlFlow::<(), _>::Continue(None)) {
            Update::Absent { value: None } => None,
            Update::Success { old } => Some(old),
            Update::Absent { value: Some(_) } | Update::Break { .. } => unreachable!(),
        }
    }

    #[inline]
    fn update_with_impl<F, B>(&mut self, key: K::Borrow<'_>, with: F) -> Update<'_, V, B>
    where
        F: FnOnce(V::Borrow<'_>) -> ControlFlow<B, Option<V>>,
    {
        let reader = K::Read::from(key);
        let mut cursor = CursorMut::<K>::new(&mut self.root, reader);

        let old = match cursor.traverse_update() {
            None => return Update::Absent { value: None },
            Some(Err(Frozen)) => unreachable!(),
            Some(Ok(old)) => old,
        };

        let new = match with(unsafe { V::borrow_from_raw(old.into_raw()) }) {
            ControlFlow::Continue(None) => Edge::DEFAULT,
            ControlFlow::Continue(Some(new)) => unsafe {
                old.with_value_unchecked(V::into_raw(new))
            },
            ControlFlow::Break(r#break) => {
                return Update::Break {
                    old: unsafe { V::borrow_from_raw(old.into_raw()) },
                    r#break,
                };
            }
        };

        cursor.edge_mut().set_packed(new);

        Update::Success {
            old: unsafe { V::from_raw(old.into_raw()) },
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

impl<'k, K, V> FromIterator<(K::Borrow<'k>, V)> for Map<K, V>
where
    K: Key,
    V: Value,
{
    fn from_iter<T: IntoIterator<Item = (K::Borrow<'k>, V)>>(iter: T) -> Self {
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
    type Item = (K, V::Borrow<'g>);
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
    type Item = (K, V::BorrowMut<'g>);
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
