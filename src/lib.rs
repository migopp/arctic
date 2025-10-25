macro_rules! validate {
    ($($tt:tt)*) => {
        if cfg!(any(feature = "validate", debug_assertions)) {
            assert!($($tt)*);
        }
    };
}

macro_rules! validate_eq {
    ($($tt:tt)*) => {
        if cfg!(any(feature = "validate", debug_assertions)) {
            assert_eq!($($tt)*);
        }
    };
}

mod byte;
pub mod concurrent;
pub(crate) mod cursor;
mod edge;
pub mod iter;
pub mod key;
mod node;
pub mod sequential;
mod smr;
pub mod stat;
mod value;

pub(crate) use edge::Edge;
pub use key::Key;
pub(crate) use node::Node;
pub use value::Value;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    Node(node::Op),
    Edge(edge::Op),
}

impl Op {
    /// Whether this operation allocates a new node.
    #[inline]
    pub fn is_allocate(self) -> bool {
        match self {
            Self::Node(node) => node.is_allocate(),
            Self::Edge(edge) => edge.is_allocate(),
        }
    }

    /// Whether this operation retires an old node.
    #[inline]
    pub fn is_retire(self) -> bool {
        matches!(self, Self::Node(_))
    }
}

/// https://users.rust-lang.org/t/compiler-hint-for-unlikely-likely-for-if-branches/62102/4
#[inline]
#[cold]
pub(crate) fn cold() {}

#[cfg(test)]
mod tests {
    use crate::concurrent::Map;
    use crate::key::Read as _;
    use crate::sequential;

    // https://users.rust-lang.org/t/testing-if-a-type-is-implementing-an-auto-trait/90871/6
    #[test]
    const fn assert_not_sync() {
        #[allow(dead_code)]
        trait AmbiguousIfSync<T> {
            const ASSERT_NOT_SYNC: () = ();
        }

        impl<T: ?Sized> AmbiguousIfSync<((), ())> for T {}
        impl<T: ?Sized + Sync> AmbiguousIfSync<()> for T {}

        const _: () = <sequential::Map<u64, u32>>::ASSERT_NOT_SYNC;
    }

    #[test]
    fn smoke() {
        let map = Map::<Vec<u8>, _>::default();
        let mut map = map.pin();
        map.insert(b"abcd", 1u32);
        assert_eq!(map.get(b"abcd"), Some(1));
    }

    #[test]
    fn smoke_u64_key() {
        let map = Map::<Vec<u8>, _>::default();
        let key = 0xdeadbeefu64.to_be_bytes();
        let mut map = map.pin();
        map.insert(&key, 1u32);
        assert_eq!(map.get(&key), Some(1));
    }

    #[test]
    fn scan_leaf() {
        let map = Map::<u64, _>::default();
        let mut map = map.pin();
        let key = 1u64;
        map.insert(key, 2u32);
        let range = map.range(1u64, 1u64).unwrap();
        assert_eq!(range.iter().collect::<Vec<_>>(), vec![(1, 2)]);
    }

    #[test]
    fn scan_node3() {
        insert_all(0u64..3);
    }

    #[test]
    fn scan_node256() {
        insert_all(0u64..256);
    }

    // #[test]
    // fn scan_node256_exclusive() {
    //     let map = insert_all(0u64..256);
    //     let mut map = map.pin();
    //     assert_eq!(
    //         map.range_non_linearizable(0, 255).collect::<Vec<_>>(),
    //         (0..256).map(|key| (key, key as u32)).collect::<Vec<_>>()
    //     );
    // }

    #[test]
    fn scan_gap() {
        let map = insert_all((0u64..512).step_by(2));
        let mut map = map.pin();
        let range = map.range(256u64, 511u64).unwrap();
        assert_eq!(
            range.iter().collect::<Vec<_>>(),
            (256..512)
                .step_by(2)
                .map(|key| (key, key as u32 / 2))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn node3_overwrite() {
        let mut map = Map::<u64, _>::default();
        let mut pin = map.pin();

        for value in [1u32, 2, 3] {
            pin.insert(1, value);
            assert_eq!(pin.get(1), Some(value));
        }

        drop(pin);
        assert_eq!(map.as_sequential().iter::<crate::iter::Sorted>().count(), 1);

        map.as_sequential()
            .iter::<crate::iter::Sorted>()
            .for_each(|(key, value)| {
                assert_eq!(key, 1);
                assert_eq!(value, 3);
            });
    }

    #[test]
    fn node3_full() {
        insert_all(0u16..3);
    }

    #[test]
    fn node3_expand() {
        insert_all(0u16..4);
    }

    #[test]
    fn node15_full() {
        insert_all(0u16..15);
    }

    #[test]
    fn node15_expand() {
        insert_all(0u16..16);
    }

    #[test]
    fn node256_full() {
        insert_all(0u16..=255);
    }

    #[test]
    fn split_edges() {
        let mut key = (0..100).collect::<Vec<_>>();
        insert_all(core::iter::from_fn(|| {
            if key.is_empty() {
                None
            } else {
                let mut next = key.clone();
                next.push(0);
                key.pop();
                Some(next)
            }
        }));
    }

    fn insert_all<I, K>(iter: I) -> Map<K, u32>
    where
        I: IntoIterator<Item = K>,
        K: crate::Key + Clone + Ord + core::fmt::Debug,
    {
        let mut keys = iter
            .into_iter()
            .enumerate()
            .map(|(index, key)| (key, index as u32))
            .collect::<Vec<_>>();

        let mut map = Map::default();
        let mut pin = map.pin();

        for (key, value) in &keys {
            pin.insert(key.borrow(), *value);
            assert_eq!(pin.get(key.borrow()), Some(*value));
        }

        for (key, value) in &keys {
            assert_eq!(pin.get(key.borrow()), Some(*value));
        }

        drop(pin);

        let mut iter = map.as_sequential().iter::<crate::iter::Sorted>();
        let mut count = 0;
        while iter.lend().is_some() {
            count += 1;
        }
        drop(iter);

        assert_eq!(count, keys.len());

        keys.sort_by(|(l, _), (r, _)| l.cmp(r));

        // Sequential iteration
        map.as_sequential()
            .iter::<crate::iter::Sorted>()
            .zip(&keys)
            .for_each(|((lk, lv), (rk, rv))| {
                assert_eq!(lk, *rk);
                assert_eq!(lv, *rv);
            });

        let mut pin = map.pin();

        let Some(((first, _), (last, _))) = keys.first().zip(keys.last()) else {
            drop(pin);
            return map;
        };

        // Concurrent prefix iteration, non-linearizable
        let prefix = pin
            .prefix(K::Read::from(first.borrow()).prefix(&K::Read::from(last.borrow())))
            .unwrap();
        prefix
            .iter::<core::iter::Rev<crate::iter::Sorted>>()
            .zip(keys.iter().rev())
            .for_each(|((lk, lv), (rk, rv))| {
                assert_eq!(lk, *rk);
                assert_eq!(lv, *rv);
            });
        drop(prefix);

        // Concurrent range iteration, non-linearizable
        let range = pin.range(first.borrow(), last.borrow()).unwrap();
        range.iter().zip(&keys).for_each(|((lk, lv), (rk, rv))| {
            assert_eq!(lk, *rk);
            assert_eq!(lv, *rv);
        });
        drop(range);

        // Concurrent iteration, linearizable
        let mut buffer = Vec::new();
        let mut range = pin
            .range_optimistic(&mut buffer, usize::MAX, first.borrow(), last.borrow())
            .unwrap();
        range.drain().zip(&keys).for_each(|((lk, lv), (rk, rv))| {
            assert_eq!(lk, *rk);
            assert_eq!(lv, *rv);
        });
        drop(range);
        drop(pin);

        map
    }
}
