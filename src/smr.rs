use core::cell::RefCell;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic64;
use thread_local::ThreadLocal;

use crate::key;
use crate::node;
use crate::node::Node15;
use crate::node::Node256;
use crate::node::Node3;
use crate::stat;
use crate::Edge;

const RETIRED_COUNT: usize = 16;

pub(crate) struct State {
    hazards: ThreadLocal<Atomic64<key::Array>>,
    retired: ThreadLocal<RefCell<Vec<(ribbit::Packed<key::Array>, u64)>>>,
}

impl State {
    pub(crate) fn protect<K: key::Iterator>(&self, key: &K) -> Guard {
        let prefix = key.prefix(key::Array::MAX_LEN);
        self.hazards
            .get_or_default()
            .store_packed(prefix, Ordering::Relaxed);
        BARRIER.fast();
        Guard { state: self }
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
        for (_, data) in self.retired.iter_mut().flat_map(RefCell::get_mut) {
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
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        self.state
            .hazards
            .get_or_default()
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

        let mut retired = self.state.retired.get_or_default().borrow_mut();
        retired.push((prefix, data));
        if retired.len() >= RETIRED_COUNT {
            drop(retired);
            self.flush();
        }
    }

    #[cold]
    fn flush(&self) {
        BARRIER.slow();

        let hazards = self
            .state
            .hazards
            .iter()
            .map(|hazard| hazard.load_packed(Ordering::Relaxed))
            .collect::<Vec<_>>();

        self.state
            .retired
            .get_or_default()
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

#[cfg(feature = "opt-membarrier")]
static BARRIER: std::sync::LazyLock<Barrier> = std::sync::LazyLock::new(|| Barrier::new().unwrap());

#[cfg(not(feature = "opt-membarrier"))]
static BARRIER: Barrier = Barrier;

/// https://pvk.ca/Blog/2020/07/07/flatter-wait-free-hazard-pointers/
struct Barrier;

impl Barrier {
    #[cfg(feature = "opt-membarrier")]
    fn new() -> std::io::Result<Self> {
        unsafe {
            match libc::syscall(
                libc::SYS_membarrier,
                libc::MEMBARRIER_CMD_REGISTER_PRIVATE_EXPEDITED,
                0,
                0,
            ) {
                0 => Ok(Self),
                _ => Err(std::io::Error::last_os_error()),
            }
        }
    }

    #[inline(always)]
    fn fast(&self) {
        if cfg!(feature = "opt-membarrier") {
            core::sync::atomic::compiler_fence(Ordering::SeqCst);
        } else {
            core::sync::atomic::fence(Ordering::SeqCst);
        }
    }

    #[inline]
    fn slow(&self) {
        #[cfg(feature = "opt-membarrier")]
        unsafe {
            match libc::syscall(
                libc::SYS_membarrier,
                libc::MEMBARRIER_CMD_PRIVATE_EXPEDITED,
                0,
                0,
            ) {
                0 => (),
                _ => panic!("membarrier: {:?}", std::io::Error::last_os_error()),
            }
        }

        #[cfg(not(feature = "opt-membarrier"))]
        core::sync::atomic::fence(Ordering::SeqCst);
    }
}
