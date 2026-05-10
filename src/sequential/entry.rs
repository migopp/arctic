use core::marker::PhantomData;
use core::ptr::NonNull;

use crate::raw::Cursor;
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
    Occupied(Occupied<'g, V>),
}

impl<'g, 'k, K: Key, V: Value + 'g> Entry<'g, 'k, K, V> {
    #[inline]
    pub fn or_insert(self, default: V) -> &'g mut V {
        match self {
            Self::Occupied(entry) => entry.into_mut(),
            Self::Vacant(entry) => entry.insert(default),
        }
    }

    #[inline]
    pub fn or_insert_with<F: FnOnce() -> V>(self, default: F) -> &'g mut V {
        match self {
            Self::Occupied(entry) => entry.into_mut(),
            Self::Vacant(entry) => entry.insert(default()),
        }
    }

    #[inline]
    pub fn and_modify<F>(self, modify: F) -> Self
    where
        F: FnOnce(&mut V),
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
    #[inline]
    pub fn or_default(self) -> &'g mut V {
        self.or_insert_with(V::default)
    }
}

pub struct Vacant<'g, 'k, K: Key, V: Value + 'g> {
    pub(super) cursor: Cursor<'g, K::Read<'k>, path::Discard>,
    pub(super) _value: PhantomData<&'g V>,
}

pub struct Occupied<'g, V: Value + 'g> {
    pub(super) value: NonNull<V>,
    pub(super) _value: PhantomData<&'g mut V>,
}

impl<'g, 'k, K: Key, V: Value + 'g> Vacant<'g, 'k, K, V> {
    #[inline]
    pub fn insert(self, value: V) -> &'g mut V {
        self.insert_entry(value).into_mut()
    }

    pub fn insert_entry(mut self, value: V) -> Occupied<'g, V> {
        let new_value = V::into_raw(value);
        loop {
            match self.cursor.traverse_insert() {
                crate::raw::cursor::Insert::Value { old_value, old } => {
                    match self.cursor.create_path(old, new_value) {
                        Err(Frozen) => unreachable!(),
                        Ok(new) => {
                            validate!(old_value.is_none());
                            unsafe {
                                self.cursor.edge_mut().set_packed(new);
                                return Occupied {
                                    value: self.cursor.as_value_unchecked().cast::<V>(),
                                    _value: PhantomData,
                                };
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

impl<'g, V: Value> Occupied<'g, V> {
    #[inline]
    pub fn get(&self) -> &V {
        unsafe { self.value.as_ref() }
    }

    #[inline]
    pub fn get_mut(&mut self) -> &mut V {
        unsafe { self.value.as_mut() }
    }

    #[inline]
    pub fn insert(&mut self, value: V) -> V {
        unsafe { core::mem::replace(self.value.as_mut(), value) }
    }

    #[inline]
    pub fn into_mut(mut self) -> &'g mut V {
        unsafe { self.value.as_mut() }
    }

    #[inline]
    pub fn and_modify<F: FnOnce(&mut V)>(mut self, modify: F) {
        modify(unsafe { self.value.as_mut() })
    }
}
