use core::hash::Hasher as _;
use std::sync::Barrier;

use arctic::raw::Key;
use arctic::Value as _;

mod u64 {
    use arctic::raw::Key;

    use super::test_map;
    use super::Workload;

    #[test]
    fn many() {
        test_map(&U64, 100, 10_000_000, false);
    }

    #[test]
    fn two() {
        test_map(&U64, 2, 10_000_000, true);
    }

    #[test]
    fn one() {
        test_map(&U64, 1, 10_000_000, true);
    }

    struct U64;

    impl Workload for U64 {
        type Key = u64;

        type Value = u64;

        fn key<'a>(&'a self, index: usize) -> <Self::Key as Key>::Borrow<'a> {
            index as u64
        }

        fn value(&self, index: usize) -> Self::Value {
            index as u64
        }

        fn validate<'a, 'g, 'l>(
            &'a self,
            index: usize,
            key: <Self::Key as Key>::Borrow<'a>,
            value: u64,
        ) where
            'a: 'g,
            'g: 'l,
        {
            assert_eq!(index as u64, key);
            assert_eq!(index as u64, value);
        }
    }
}

mod boxed {
    use arctic::raw::Key;

    use super::test_map;
    use super::Workload;

    struct Boxed;

    #[test]
    fn many() {
        test_map(&Boxed, 100, 10_000_000, false);
    }

    #[test]
    fn two() {
        test_map(&Boxed, 2, 10_000_000, false);
    }

    #[test]
    fn one() {
        test_map(&Boxed, 1, 10_000_000, false);
    }

    #[derive(Debug, PartialEq, Eq)]
    struct Entry {
        key: u32,
        value: u64,
    }

    impl Entry {
        fn new(index: usize) -> Self {
            Self {
                key: index as u32,
                value: index as u64 + 1,
            }
        }
    }

    impl Workload for Boxed {
        type Key = u32;

        type Value = Box<Entry>;

        fn key<'a>(&'a self, index: usize) -> <Self::Key as Key>::Borrow<'a> {
            index as u32
        }

        fn value(&self, index: usize) -> Self::Value {
            Box::new(Entry::new(index))
        }

        fn validate<'a, 'g, 'l>(
            &'a self,
            index: usize,
            key: <Self::Key as Key>::Borrow<'a>,
            value: &Entry,
        ) where
            'a: 'g,
            'g: 'l,
        {
            assert_eq!(key, index as u32);
            assert_eq!(*value, Entry::new(index));
        }
    }
}

trait Workload: Sized + Sync {
    type Key: arctic::concurrent::Key + Sync;

    type Value: arctic::Value + Send + Sync;

    fn key<'a>(&'a self, index: usize) -> <Self::Key as Key>::Borrow<'a>;

    fn value(&self, index: usize) -> Self::Value;

    fn validate<'a, 'g, 'l>(
        &'a self,
        index: usize,
        key: <Self::Key as Key>::Borrow<'a>,
        value: <Self::Value as arctic::sequential::Value>::Borrow<'_>,
    ) where
        'a: 'g,
        'g: 'l;
}

fn test_map<'k, K: Workload>(key_set: &'k K, thread_count: usize, key_count: usize, hash: bool)
where
    for<'a> <K::Key as Key>::Borrow<'a>: Sync,
{
    assert_eq!(key_count % thread_count, 0);

    let barrier = &Barrier::new(thread_count);
    let items = if hash {
        let mut indices = (0..key_count)
            .map(|index| {
                let mut hasher = rapidhash::fast::RapidHasher::default_const();
                hasher.write_usize(index);
                hasher.finish() as usize
            })
            .collect::<Vec<_>>();
        indices.sort_unstable();
        indices.dedup();
        indices
            .into_iter()
            .map(|index| (index, key_set.key(index)))
            .collect::<Vec<_>>()
    } else {
        (0..key_count)
            .map(|index| (index, key_set.key(index)))
            .collect::<Vec<_>>()
    };

    let map = &arctic::concurrent::Map::<K::Key, _>::default();

    std::thread::scope(|scope| {
        for chunk in items.chunks_exact(key_count / thread_count) {
            scope.spawn(move || {
                let mut map = map.pin();

                barrier.wait();

                for (index, key) in chunk {
                    let value = key_set.value(*index);
                    let old = map.upsert(*key, value);
                    assert!(old.is_none());
                }

                barrier.wait();

                for (index, key) in chunk.iter().take(chunk.len() / 2) {
                    let value = map.remove(*key).unwrap();
                    key_set.validate(*index, *key, K::Value::borrow_owned(&value));
                }

                barrier.wait();

                for (index, key) in chunk.iter().skip(chunk.len() / 2) {
                    let value = map.get(*key);
                    key_set.validate(*index, *key, K::Value::borrow_shared(&value.unwrap()));
                }
            });
        }
    });
}
