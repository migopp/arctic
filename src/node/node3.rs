use core::sync::atomic::Ordering;

use ribbit::atomic::A128;
use ribbit::u24;
use ribbit::u3;

use crate::node::Slot;
use crate::Node;

pub(crate) struct Node3 {
    header: A128<Header>,
    slots: [A128<Slot>; 3],
}

impl Node for Node3 {
    fn get(&self, key: u8) -> Option<&A128<Slot>> {
        let header = self.header.load(Ordering::Relaxed);
        let map = header.map();
        let keys = map.keys().value();
        let valid = map.valid().value();

        for i in 0..3 {
            if valid & (1u8 << i) != 1 {
                continue;
            }

            if (keys >> (i * 8)) as u8 != key {
                continue;
            }

            return Some(&self.slots[i]);
        }

        None
    }
}

#[ribbit::pack(size = 128)]
struct Header {
    freeze: bool,
    grow: bool,
    #[ribbit(size = 32)]
    map: Map,
}

#[ribbit::pack(size = 32)]
struct Map {
    valid: u3,
    keys: u24,
}
