use core::cell::RefCell;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic64;
use thread_local::ThreadLocal;

use crate::key;
use crate::membarrier;
use crate::node;
use crate::node::Node15;
use crate::node::Node256;
use crate::node::Node3;
use crate::stat;
use crate::Edge;

const RETIRED_COUNT: usize = 16;

#[repr(C, align(64))]
#[derive(Default)]
struct Cache<T>(T);

pub(crate) struct State {
    hazards: ThreadLocal<Cache<Atomic64<key::Array>>>,
    retired: ThreadLocal<Cache<RefCell<Vec<(ribbit::Packed<key::Array>, u64)>>>>,
}

impl State {
    #[inline]
    pub(crate) fn protect<K: key::Iterator>(&self, key: &K) -> Guard {
        let prefix = key.prefix(key::Array::MAX_LEN);
        let hazard = &self.hazards.get_or_default().0;
        hazard.store_packed(prefix, Ordering::Relaxed);
        membarrier::fast();
        Guard {
            state: self,
            hazard,
        }
    }
}

impl Default for State {
    fn default() -> Self {
        Self {
            hazards: ThreadLocal::with_capacity(128),
            retired: ThreadLocal::with_capacity(128),
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        for (_, data) in self
            .retired
            .iter_mut()
            .map(|Cache(retired)| retired)
            .flat_map(RefCell::get_mut)
        {
            let tag = *data & 0b11u64;
            let data = *data & !0b11u64;
            match tag {
                0 => drop(unsafe { Box::from_raw(data as *mut Node3) }),
                1 => drop(unsafe { Box::from_raw(data as *mut Node15) }),
                2 => drop(unsafe { Box::from_raw(data as *mut Node256) }),
                _ => unreachable!(),
            }
        }
    }
}

pub(crate) struct Guard<'a> {
    state: &'a State,
    hazard: &'a Atomic64<key::Array>,
}

impl Drop for Guard<'_> {
    #[inline]
    fn drop(&mut self) {
        self.hazard
            .store_packed(key::Array::EMPTY, Ordering::Relaxed);
    }
}

impl Guard<'_> {
    pub(crate) unsafe fn retire(
        &self,
        prefix: ribbit::Packed<key::Array>,
        edge: ribbit::Packed<Edge>,
    ) {
        let kind = edge.meta().kind();
        let tag = if kind < node::Kind::NODE_3 {
            return;
        } else if kind == node::Kind::NODE_3 {
            0
        } else if kind == node::Kind::NODE_15 {
            1
        } else {
            validate_eq!(kind, node::Kind::NODE_256);
            2
        };

        stat::increment(stat::Counter::Retire);
        let data = edge.data() | tag;

        let mut retired = self.state.retired.get_or_default().0.borrow_mut();
        retired.push((prefix, data));
        if retired.len() >= RETIRED_COUNT {
            drop(retired);
            self.flush();
        }
    }

    #[cold]
    fn flush(&self) {
        membarrier::slow();

        let hazards = self
            .state
            .hazards
            .iter()
            .map(|hazard| hazard.0.load_packed(Ordering::Relaxed))
            .collect::<Vec<_>>();

        self.state
            .retired
            .get_or_default()
            .0
            .borrow_mut()
            .retain(|(key, data)| {
                if hazards
                    .iter()
                    .any(|hazard| key::Array::has_prefix(*hazard, *key))
                {
                    return true;
                }

                let tag = data & 0b11u64;
                let data = data & !0b11u64;
                match tag {
                    0 => drop(unsafe { Box::from_raw(data as *mut Node3) }),
                    1 => drop(unsafe { Box::from_raw(data as *mut Node15) }),
                    2 => drop(unsafe { Box::from_raw(data as *mut Node256) }),
                    _ => unreachable!(),
                }

                false
            })
    }
}
