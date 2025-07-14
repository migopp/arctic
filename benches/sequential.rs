use std::sync::Arc;

use bustle::Collection;
use bustle::CollectionHandle;
use bustle::Mix;
use bustle::Workload;

struct Art(Arc<art::Map<u64, u32>>);

impl Collection for Art {
    type Handle = Art;

    fn with_capacity(_capacity: usize) -> Self {
        Self(Arc::new(art::Map::default()))
    }

    fn pin(&self) -> Self::Handle {
        Self(Arc::clone(&self.0))
    }
}

impl CollectionHandle for Art {
    type Key = u64;

    fn get(&mut self, key: &Self::Key) -> bool {
        match self.0.get(*key) {
            Some(value) => {
                assert_eq!(*key as u32, value);
                true
            }
            None => false,
        }
    }

    fn insert(&mut self, key: &Self::Key) -> bool {
        match self.0.insert(*key, *key as u32) {
            None => true,
            Some(value) => {
                assert_eq!(*key as u32, value);
                false
            }
        }
    }

    fn remove(&mut self, key: &Self::Key) -> bool {
        self.0.remove(*key).is_some()
    }

    fn update(&mut self, key: &Self::Key) -> bool {
        self.0.update(*key, 0).is_some()
    }
}

fn main() {
    Workload::new(
        1,
        Mix {
            read: 100,
            insert: 0,
            remove: 0,
            update: 0,
            upsert: 0,
        },
    )
    .initial_capacity_log2(16)
    .prefill_fraction(1.0)
    .seed(core::array::from_fn(|i| i as u8))
    .run::<Art>();
}
