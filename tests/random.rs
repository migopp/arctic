use core::ops::Range;
use std::sync::Barrier;

use rand::seq::SliceRandom as _;

#[test]
fn many() {
    test_map(128, 1_000_000, true);
}

#[test]
fn two() {
    test_map(2, 1_000_000, true);
}

#[test]
fn one() {
    test_map(1, 1_000_000, true);
}

fn test_map(thread_count: usize, key_count: u32, shuffle: bool) {
    let barrier = &Barrier::new(thread_count);
    let map = &arctic::concurrent::Map::<u64, u32>::default();

    std::thread::scope(|scope| {
        for thread_id in 0..thread_count {
            scope.spawn(move || {
                let keys = generate(thread_count, thread_id, 0..key_count, shuffle);
                let mut map = map.pin();

                barrier.wait();

                for key in keys {
                    assert_eq!(map.insert(&(key as u64), key), None);
                }
            });
        }
    });

    std::thread::scope(|scope| {
        for thread_id in 0..thread_count {
            scope.spawn(move || {
                let keys = generate(thread_count, thread_id, 0..key_count / 2, shuffle);
                let mut map = map.pin();

                barrier.wait();

                for key in keys {
                    assert_eq!(map.remove(&(key as u64)), Some(key));
                }
            });
        }
    });

    std::thread::scope(|scope| {
        for thread_id in 0..thread_count {
            scope.spawn(move || {
                let keys = generate(thread_count, thread_id, key_count / 2..key_count, shuffle);
                let map = map.pin();

                barrier.wait();

                for key in keys {
                    assert_eq!(map.get(&(key as u64)), Some(key));
                }
            });
        }
    });
}

fn generate(
    thread_count: usize,
    thread_id: usize,
    key_range: Range<u32>,
    shuffle: bool,
) -> Vec<u32> {
    let mut keys = key_range
        .map(|key| key * thread_count as u32 + thread_id as u32)
        .collect::<Vec<_>>();

    if shuffle {
        let mut rng = rand::rng();
        keys.shuffle(&mut rng);
    }

    keys
}
