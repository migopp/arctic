use core::marker::PhantomData;
use core::ptr::NonNull;

use crate::raw::Cursor;
use crate::raw::Edge;
use crate::raw::Frozen;
use crate::raw::Key;
use crate::raw::cursor::path;
use crate::raw::edge::Meta as _;
use crate::sequential::Value;
use crate::stat;

pub enum Entry<'g, 'k, K, V>
where
    K: Key,
    V: Value + 'g,
{
    Vacant(Vacant<'g, 'k, K, V>),
    Occupied(Occupied<'g, K, V>),
}

impl<'g, 'k, K: Key, V: Value + 'g> Entry<'g, 'k, K, V> {
    pub fn or_insert(self, default: V) -> &'g mut V::Target {
        match self {
            Self::Occupied(entry) => entry.into_mut(),
            Self::Vacant(entry) => entry.insert(default),
        }
    }

    pub fn or_insert_with<F: FnOnce() -> V>(self, default: F) -> &'g mut V::Target {
        match self {
            Self::Occupied(entry) => entry.into_mut(),
            Self::Vacant(entry) => entry.insert(default()),
        }
    }

    pub fn and_modify<F>(self, modify: F) -> Self
    where
        F: FnOnce(&mut V::Target),
    {
        match self {
            Self::Occupied(mut entry) => {
                modify(entry.get_mut());
                Self::Occupied(entry)
            }
            Self::Vacant(entry) => Self::Vacant(entry),
        }
    }
}

impl<'g, 'k, K: Key, V: Value + Default + 'g> Entry<'g, 'k, K, V> {
    pub fn or_default(self) -> &'g mut V::Target {
        self.or_insert_with(V::default)
    }
}

pub struct Vacant<'g, 'k, K: Key, V: Value + 'g> {
    pub(super) cursor: Cursor<'g, K::Read<'k>, path::Discard>,
    pub(super) _value: PhantomData<&'g V>,
}

pub struct Occupied<'g, K: Key, V: Value + 'g> {
    pub(super) edge: &'g mut ribbit::Atomic<Edge<K::Edge>>,
    pub(super) _value: PhantomData<&'g V>,
}

impl<'g, 'k, K: Key, V: Value + 'g> Vacant<'g, 'k, K, V> {
    pub fn insert(mut self, value: V) -> &'g mut V::Target {
        let new_value = V::into_raw(value);
        loop {
            match self.cursor.traverse_insert() {
                crate::raw::cursor::Insert::Value { old_value, old } => {
                    match self.cursor.create_path(old, new_value) {
                        Err(Frozen) => unreachable!(),
                        Ok(new) => {
                            validate!(old_value.is_none());
                            unsafe {
                                let edge = self.cursor.edge_mut();
                                edge.set_packed(new);
                                return V::target_mut_from_raw(Edge::as_value_mut_unchecked(
                                    NonNull::from(edge),
                                ));
                            };
                        }
                    }
                }
                crate::raw::cursor::Insert::Replace { old_node, old } => {
                    validate!(!old.meta().is_frozen());
                    let (_smo, new) = unsafe { old_node.replace::<false>(old.meta()) };
                    unsafe { self.cursor.edge_mut() }.set_packed(new);
                    if let Some(node) = old.as_node() {
                        unsafe { node.deallocate(stat::Counter::FreeRetire) };
                    }
                }
            }
        }
    }
}

impl<'g, K: Key, V: Value> Occupied<'g, K, V> {
    pub fn get(&self) -> &V::Target {
        unsafe { V::target_from_raw(Edge::as_value_unchecked(NonNull::from(&*self.edge))) }
    }

    pub fn get_mut(&mut self) -> &mut V::Target {
        unsafe {
            V::target_mut_from_raw(Edge::as_value_mut_unchecked(NonNull::from(&mut *self.edge)))
        }
    }

    pub fn insert(self, value: V) -> V {
        let new = V::into_raw(value);
        unsafe {
            let old = self.edge.get_packed();
            self.edge.set_packed(Edge::new_value(old.meta(), new));
            V::from_raw(old.into_value_unchecked())
        }
    }

    pub fn into_mut(self) -> &'g mut V::Target {
        unsafe {
            V::target_mut_from_raw(Edge::as_value_mut_unchecked(NonNull::from(&mut *self.edge)))
        }
    }

    pub fn and_modify<F: FnOnce(&mut V::Target)>(self, modify: F) {
        modify(unsafe {
            V::target_mut_from_raw(Edge::as_value_mut_unchecked(NonNull::from(&*self.edge)))
        })
    }
}
