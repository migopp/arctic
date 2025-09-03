mod cursor;
mod edge;
mod key;
mod node;
mod raw;
pub mod stat;

pub use raw::Raw;
use ribbit::u48;

use core::marker::PhantomData;

pub(crate) use edge::Edge;
pub(crate) use node::Node;

pub struct Map<K, V> {
    raw: Raw,
    _key: PhantomData<K>,
    _value: PhantomData<V>,
}

impl<K, V> Default for Map<K, V> {
    fn default() -> Self {
        Self {
            raw: Raw::default(),
            _key: PhantomData,
            _value: PhantomData,
        }
    }
}

impl<K: Key, V: Value> Map<K, V> {
    pub fn get(&self, key: K) -> Option<V> {
        let key = key.to_byte_array();
        let key = key.as_ref();
        self.raw.get(key).map(V::from_u48)
    }

    pub fn insert(&self, key: K, value: V) -> Option<V> {
        let key = key.to_byte_array();
        let key = key.as_ref();
        self.raw.insert(key, value.into_u48()).map(V::from_u48)
    }

    pub fn remove(&self, key: K) -> Option<V> {
        let key = key.to_byte_array();
        let key = key.as_ref();
        self.raw.remove(key).map(V::from_u48)
    }

    pub fn update(&self, key: K, value: V) -> Option<V> {
        let key = key.to_byte_array();
        let key = key.as_ref();
        self.raw.update(key, value.into_u48()).map(V::from_u48)
    }
}

pub trait Key {
    type ByteArray: AsRef<[u8]>;
    fn to_byte_array(&self) -> Self::ByteArray;
}

impl Key for u64 {
    type ByteArray = [u8; 8];
    fn to_byte_array(&self) -> Self::ByteArray {
        self.to_be_bytes()
    }
}

impl Key for u32 {
    type ByteArray = [u8; 4];
    fn to_byte_array(&self) -> Self::ByteArray {
        self.to_be_bytes()
    }
}

impl Key for u16 {
    type ByteArray = [u8; 2];
    fn to_byte_array(&self) -> Self::ByteArray {
        self.to_be_bytes()
    }
}

impl Key for u8 {
    type ByteArray = [u8; 1];
    fn to_byte_array(&self) -> Self::ByteArray {
        self.to_be_bytes()
    }
}

impl<'a> Key for &'a str {
    type ByteArray = &'a [u8];
    fn to_byte_array(&self) -> Self::ByteArray {
        self.as_bytes()
    }
}

impl<'a> Key for &'a [u8] {
    type ByteArray = &'a [u8];
    fn to_byte_array(&self) -> Self::ByteArray {
        self
    }
}

impl<const LEN: usize> Key for [u8; LEN] {
    type ByteArray = Self;

    #[inline]
    fn to_byte_array(&self) -> Self::ByteArray {
        *self
    }
}

impl<const LEN: usize> Key for &'_ [u8; LEN] {
    type ByteArray = Self;
    #[inline]
    fn to_byte_array(&self) -> Self::ByteArray {
        *self
    }
}

pub trait Value {
    fn from_u48(value: u48) -> Self;
    fn into_u48(self) -> u48;
}

impl Value for u32 {
    fn from_u48(value: u48) -> Self {
        value.value() as u32
    }

    fn into_u48(self) -> u48 {
        u48::from(self)
    }
}

impl Value for () {
    fn from_u48(_: u48) -> Self {}

    fn into_u48(self) -> u48 {
        u48::new(0)
    }
}

#[cfg(test)]
mod tests {
    use crate::Map;

    #[test]
    fn smoke() {
        let art = Map::default();
        art.insert(b"abcd", 1);
        assert_eq!(art.get(b"abcd"), Some(1));
    }

    #[test]
    fn smoke_u64_key() {
        let art = Map::default();
        let key = 0xdeadbeefu64.to_be_bytes();
        art.insert(&key, 1);
        assert_eq!(art.get(&key), Some(1));
    }

    #[test]
    fn node3_overwrite() {
        let art = Map::default();

        for value in [1, 2, 3] {
            art.insert(&[1], value);
            assert_eq!(art.get(&[1]), Some(value));
        }
    }

    #[test]
    fn node3_full() {
        let art = Map::default();

        const KEYS: [u8; 3] = [1, 2, 3];

        for key in KEYS {
            art.insert(&[key], key as u32);
            assert_eq!(art.get(&[key]), Some(key as u32));
        }

        for key in KEYS {
            assert_eq!(art.get(&[key]), Some(key as u32));
        }
    }

    #[test]
    fn node3_expand() {
        let art = Map::default();

        const KEYS: [u8; 4] = [1, 2, 3, 4];

        for key in KEYS {
            art.insert(&[key], key as u32);
            assert_eq!(art.get(&[key]), Some(key as u32));
        }

        for key in KEYS {
            assert_eq!(art.get(&[key]), Some(key as u32));
        }
    }

    #[test]
    fn node256_full() {
        let art = Map::default();

        for key in 0..=255 {
            art.insert(&[key], key as u32);
            assert_eq!(art.get(&[key]), Some(key as u32));
        }

        for key in 0..=255 {
            assert_eq!(art.get(&[key]), Some(key as u32));
        }
    }
}
