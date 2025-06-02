use std::sync::Arc;

use bustle::Collection;
use bustle::CollectionHandle;
use bustle::Mix;
use bustle::Workload;

struct Art(Arc<art::Art>);

impl Collection for Art {
    type Handle = Art;

    fn with_capacity(_capacity: usize) -> Self {
        Self(Arc::new(art::Art::default()))
    }

    fn pin(&self) -> Self::Handle {
        Self(Arc::clone(&self.0))
    }
}

impl CollectionHandle for Art {
    type Key = u64;

    fn get(&mut self, key: &Self::Key) -> bool {
        self.0.get(&key.to_be_bytes()).is_some()
    }

    fn insert(&mut self, key: &Self::Key) -> bool {
        self.0.insert(&key.to_be_bytes(), 0).is_none()
    }

    fn remove(&mut self, key: &Self::Key) -> bool {
        self.0.remove(&key.to_be_bytes()).is_some()
    }

    fn update(&mut self, key: &Self::Key) -> bool {
        self.0.update(&key.to_be_bytes(), 0).is_some()
    }
}

fn main() {
    Workload::new(1, Mix::read_heavy())
        .seed(core::array::from_fn(|i| i as u8))
        .run::<Art>();
}
