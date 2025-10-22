use core::hash::Hasher as _;
use std::sync::Barrier;

mod u64 {
    use super::test_map;
    use super::Workload;

    #[test]
    fn many() {
        test_map(&U64, 128, 1_000_000, false);
    }

    #[test]
    fn two() {
        test_map(&U64, 2, 1_000_000, true);
    }

    #[test]
    fn one() {
        test_map(&U64, 1, 1_000_000, true);
    }

    struct U64;

    impl Workload for U64 {
        type Key = u64;

        type Value<'a>
            = u64
        where
            Self: 'a;

        fn key<'a>(&'a self, index: usize) -> <Self::Key as arctic::Key>::Borrow<'a> {
            index as u64
        }

        fn value<'a>(&'a self, index: usize) -> Self::Value<'a> {
            index as u64
        }

        fn validate<'a, 'g, 'l, const RETIRE: bool>(
            &'a self,
            index: usize,
            key: <Self::Key as arctic::Key>::Borrow<'a>,
            value: <Self::Value<'a> as arctic::Value>::Guard<'g, 'l, RETIRE>,
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
    use super::test_map;
    use super::Workload;

    struct Boxed;

    #[test]
    fn two() {
        test_map(&Boxed, 2, 1_000_000, false);
    }

    #[test]
    fn one() {
        test_map(&Boxed, 1, 1_000_000, false);
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

        type Value<'a> = Box<Entry>;

        fn key<'a>(&'a self, index: usize) -> <Self::Key as arctic::Key>::Borrow<'a> {
            index as u32
        }

        fn value<'a>(&'a self, index: usize) -> Self::Value<'a> {
            Box::new(Entry::new(index))
        }

        fn validate<'a, 'g, 'l, const RETIRE: bool>(
            &'a self,
            index: usize,
            key: <Self::Key as arctic::Key>::Borrow<'a>,
            value: <Self::Value<'a> as arctic::Value>::Guard<'g, 'l, RETIRE>,
        ) where
            'a: 'g,
            'g: 'l,
        {
            assert_eq!(key, index as u32);
            assert_eq!(*value.as_ref(), Entry::new(index));
        }
    }
}

trait Workload: Sized + Sync {
    type Key: arctic::Key + Sync;

    type Value<'a>: arctic::Value + Send + Sync
    where
        Self: 'a;

    fn key<'a>(&'a self, index: usize) -> <Self::Key as arctic::Key>::Borrow<'a>;

    fn value<'a>(&'a self, index: usize) -> Self::Value<'a>;

    fn validate<'a, 'g, 'l, const RETIRE: bool>(
        &'a self,
        index: usize,
        key: <Self::Key as arctic::Key>::Borrow<'a>,
        value: <Self::Value<'a> as arctic::Value>::Guard<'g, 'l, RETIRE>,
    ) where
        'a: 'g,
        'g: 'l;
}

fn test_map<'k, K: Workload>(
    key_set: &'k K,
    thread_count: usize,
    key_count_per_thread: usize,
    hash: bool,
) where
    for<'a> <K::Key as arctic::Key>::Borrow<'a>: Sync,
{
    let barrier = &Barrier::new(thread_count);
    let items = if hash {
        let mut indices = (0..key_count_per_thread * thread_count)
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
        (0..key_count_per_thread * thread_count)
            .map(|index| (index, key_set.key(index)))
            .collect::<Vec<_>>()
    };

    let map = &arctic::concurrent::Map::<K::Key, _>::default();

    std::thread::scope(|scope| {
        for chunk in items.chunks_exact(key_count_per_thread) {
            scope.spawn(move || {
                let mut map = map.pin();

                barrier.wait();

                for (index, key) in chunk {
                    let value = key_set.value(*index);
                    let old = map.insert(*key, value);
                    assert!(old.is_none());
                }

                barrier.wait();

                for (index, key) in chunk.iter().take(chunk.len() / 2) {
                    let value = map.remove(*key);
                    key_set.validate(*index, *key, value.unwrap());
                }

                barrier.wait();

                for (index, key) in chunk.iter().skip(chunk.len() / 2) {
                    let value = map.get(*key);
                    key_set.validate(*index, *key, value.unwrap());
                }
            });
        }
    });
}
