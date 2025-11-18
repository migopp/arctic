use core::cell::Cell;
use core::cell::RefCell;
use core::cmp::Reverse;
use core::num::NonZeroU64;
use core::sync::atomic::AtomicU64;
use core::sync::atomic::Ordering;
use std::collections::BinaryHeap;
use std::sync::Mutex;

use crate::concurrent::hazard::prefix;

thread_local! {
    static ID_FAST: Cell<Option<IdFast>> = const { Cell::new(None) };
    static ID_SLOW: RefCell<Option<IdSlow>> = const { RefCell::new(None) };
}

/// FIXME: dynamically allocate
const THREAD_COUNT: usize = 256;

#[repr(C, align(64))]
#[derive(Default)]
struct Cache<T>(T);

pub(super) struct Global {
    hazards: [Cache<ribbit::Atomic<prefix::Be>>; THREAD_COUNT],
    retires: [Cache<RefCell<Vec<(ribbit::Packed<prefix::Be>, u64)>>>; THREAD_COUNT],
}

unsafe impl Sync for Global {}

impl Default for Global {
    fn default() -> Self {
        Self {
            hazards: core::array::from_fn(|_| {
                Cache(ribbit::Atomic::new_packed(prefix::Be::HAZARD_NULL))
            }),
            retires: core::array::from_fn(|_| Cache::default()),
        }
    }
}

impl Global {
    pub(super) fn init_thread() {
        id();
    }

    pub(super) fn hazard(&self) -> &ribbit::Atomic<prefix::Be> {
        let id = id();
        validate!((id.0.get() as usize) < THREAD_COUNT);
        unsafe { &self.hazards.get_unchecked(id.0.get() as usize).0 }
    }

    pub(super) fn hazards(&self) -> impl Iterator<Item = &ribbit::Atomic<prefix::Be>> {
        let len = ID_HEAP.next.load(Ordering::Relaxed) as usize;
        self.hazards[1..len].iter().map(|Cache(hazard)| hazard)
    }

    pub(super) fn retires(&self) -> &RefCell<Vec<(ribbit::Packed<prefix::Be>, u64)>> {
        let id = id();
        unsafe { &self.retires.get_unchecked(id.0.get() as usize).0 }
    }

    pub(super) fn retires_mut(
        &mut self,
    ) -> impl Iterator<Item = &mut (ribbit::Packed<prefix::Be>, u64)> {
        let len = ID_HEAP.next.load(Ordering::Relaxed) as usize;
        self.retires[1..len]
            .iter_mut()
            .map(|Cache(retires)| retires)
            .flat_map(RefCell::get_mut)
    }
}

fn id() -> IdFast {
    if let Some(id) = ID_FAST.get() {
        return id;
    }

    #[cold]
    fn id_slow() -> IdFast {
        let id = ID_HEAP.allocate();
        validate!(ID_FAST.get().is_none());
        validate!(ID_SLOW.with_borrow(|slow| slow.is_none()));

        let fast = IdFast(id);
        ID_FAST.set(Some(fast));
        ID_SLOW.with_borrow_mut(|slow| *slow = Some(IdSlow(id)));
        fast
    }

    id_slow()
}

#[derive(Copy, Clone)]
struct IdFast(NonZeroU64);

#[repr(transparent)]
struct IdSlow(NonZeroU64);

impl Drop for IdSlow {
    fn drop(&mut self) {
        ID_HEAP.heap.lock().unwrap().push(Reverse(self.0));
    }
}

static ID_HEAP: IdHeap = IdHeap {
    next: AtomicU64::new(1),
    heap: Mutex::new(BinaryHeap::new()),
};

struct IdHeap {
    next: AtomicU64,
    heap: Mutex<BinaryHeap<Reverse<NonZeroU64>>>,
}

impl IdHeap {
    fn allocate(&self) -> NonZeroU64 {
        {
            let mut heap = self.heap.lock().unwrap();
            if let Some(Reverse(id)) = heap.pop() {
                return id;
            }
        }

        let next = self.next.fetch_add(1, Ordering::Relaxed);
        NonZeroU64::new(next).unwrap()
    }
}
