mod cursor;
mod key;
mod node;
mod slot;

use core::sync::atomic::Ordering;

use cursor::Cursor;
use cursor::Op;
pub(crate) use node::Node;
use ribbit::atomic::A128;
use ribbit::u48;
use ribbit::unpack;
pub(crate) use slot::Slot;

pub struct Art {
    root: A128<Slot>,
}

impl Default for Art {
    fn default() -> Self {
        Art {
            root: A128::new(Slot::default()),
        }
    }
}

impl Art {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, key: &[u8], value: u64) -> Option<u64> {
        match self.insert_optimistic(key, value) {
            Ok(old) => old,
            Err(()) => self.insert_pessimistic(key, value),
        }
    }

    #[inline]
    fn insert_optimistic(&self, key: &[u8], value: u64) -> Result<Option<u64>, ()> {
        self.insert_impl::<cursor::Optimistic>(key, value)
    }

    #[cold]
    fn insert_pessimistic(&self, key: &[u8], value: u64) -> Option<u64> {
        self.insert_impl::<cursor::Pessimistic>(key, value).unwrap()
    }

    fn insert_impl<'a, P: cursor::History<'a>>(
        &'a self,
        key: &[u8],
        value: u64,
    ) -> Result<Option<u64>, P::PopError> {
        let value = u48::new(value);
        let mut cursor = Cursor::<P>::new(&self.root, key);

        loop {
            let (op, old, new) = cursor.traverse_strong(value);

            let conflict = match cursor.here().compare_exchange(
                old.with_frozen(false),
                new,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(old) if matches!(op, Op::Slot(slot::Op::Insert)) => {
                    return match old.kind().unpack() {
                        <unpack![node::Kind]>::Invalid => Ok(None),
                        <unpack![node::Kind]>::Valid => Ok(Some(u64::from(old.next()))),
                        _ => unreachable!(),
                    };
                }
                // FIXME: retire old allocation with SMR
                Ok(_) => continue,
                Err(conflict) => conflict,
            };

            match op {
                Op::Node(node::Op::Destroy | node::Op::Compress)
                | Op::Slot(slot::Op::Insert | slot::Op::Remove) => (),

                Op::Node(node::Op::Grow | node::Op::Replace | node::Op::Shrink)
                | Op::Slot(slot::Op::Create | slot::Op::Expand) => unsafe { new.deallocate() },
            }

            if conflict.frozen() {
                cursor.pop(conflict.grow())?;
            }
        }
    }

    pub fn get(&self, key: &[u8]) -> Option<u64> {
        let mut cursor = Cursor::<cursor::Optimistic>::new(&self.root, key);
        let snapshot = cursor.traverse_weak()?;

        match snapshot.kind().unpack() {
            <unpack![node::Kind]>::Invalid => None,
            <unpack![node::Kind]>::Valid => Some(u64::from(snapshot.next())),
            _ => unreachable!(),
        }
    }

    pub fn remove(&self, key: &[u8]) -> Option<u64> {
        let mut cursor = Cursor::<cursor::Optimistic>::new(&self.root, key);
        let mut snapshot = cursor.traverse_weak()?;
        let slot = cursor.here();

        loop {
            match slot.compare_exchange(
                snapshot.with_frozen(false),
                snapshot
                    .with_frozen(false)
                    .with_kind(node::Kind::new(<unpack![node::Kind]>::Invalid)),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(conflict) if conflict.frozen() => {
                    todo!()
                }
                Err(conflict) if conflict.key() != snapshot.key() => todo!(),
                Err(conflict) => {
                    snapshot = conflict;
                }
            }
        }

        match snapshot.kind().unpack() {
            <unpack![node::Kind]>::Invalid => None,
            <unpack![node::Kind]>::Valid => Some(u64::from(snapshot.next())),
            _ => unreachable!(),
        }
    }

    pub fn update(&self, key: &[u8], value: u64) -> Option<u64> {
        let mut cursor = Cursor::<cursor::Optimistic>::new(&self.root, key);
        let mut snapshot = cursor.traverse_weak()?;
        let slot = cursor.here();

        loop {
            match slot.compare_exchange(
                snapshot.with_frozen(false),
                snapshot
                    .with_frozen(false)
                    .with_kind(node::Kind::new(<unpack![node::Kind]>::Valid))
                    .with_next(u48::new(value)),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(conflict) if conflict.frozen() => todo!(),
                Err(conflict) if conflict.key() != snapshot.key() => todo!(),
                Err(conflict) => {
                    snapshot = conflict;
                }
            }
        }

        match snapshot.kind().unpack() {
            <unpack![node::Kind]>::Invalid => None,
            <unpack![node::Kind]>::Valid => Some(u64::from(snapshot.next())),
            _ => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Art;

    #[test]
    fn smoke() {
        let art = Art::default();
        art.insert(b"abcd", 1);
        assert_eq!(art.get(b"abcd"), Some(1));
    }

    #[test]
    fn smoke_u64_key() {
        let art = Art::default();
        let key = 0xdeadbeefu64.to_be_bytes();
        art.insert(&key, 1);
        assert_eq!(art.get(&key), Some(1));
    }

    #[test]
    fn node3_overwrite() {
        let art = Art::default();

        for value in [1, 2, 3] {
            art.insert(&[1], value as u64);
            assert_eq!(art.get(&[1]), Some(value as u64));
        }
    }

    #[test]
    fn node3_full() {
        let art = Art::default();

        const KEYS: [u8; 3] = [1, 2, 3];

        for key in KEYS {
            art.insert(&[key], key as u64);
            assert_eq!(art.get(&[key]), Some(key as u64));
        }

        for key in KEYS {
            assert_eq!(art.get(&[key]), Some(key as u64));
        }
    }

    #[test]
    fn node3_expand() {
        let art = Art::default();

        const KEYS: [u8; 4] = [1, 2, 3, 4];

        for key in KEYS {
            art.insert(&[key], key as u64);
            assert_eq!(art.get(&[key]), Some(key as u64));
        }

        for key in KEYS {
            assert_eq!(art.get(&[key]), Some(key as u64));
        }
    }

    #[test]
    fn node256_full() {
        let art = Art::default();

        for key in 0..=255 {
            art.insert(&[key], key as u64);
            assert_eq!(art.get(&[key]), Some(key as u64));
        }

        for key in 0..=255 {
            assert_eq!(art.get(&[key]), Some(key as u64));
        }
    }
}
