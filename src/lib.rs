use core::sync::atomic::AtomicPtr;
use core::sync::atomic::Ordering;

#[derive(Default)]
pub struct Art {
    root: Ptr<u64>,
}

impl Art {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, key: &[u8], value: u64) {}

    pub fn get(&self, key: &[u8]) -> Option<u64> {
        None
    }
}

#[derive(Default)]
struct Ptr<T>(AtomicPtr<T>);

impl<T> Ptr<T> {
    const LEAF: usize = 1 << 63;

    fn leaf(pointer: *mut T) -> Self {
        Self(AtomicPtr::new(
            pointer.map_addr(|address| address | Self::LEAF),
        ))
    }

    fn is_leaf(&self) -> bool {
        self.0.load(Ordering::Relaxed).addr() & Self::LEAF > 0
    }
}
