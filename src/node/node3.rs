use core::sync::atomic::Ordering;

use ribbit::atomic::A128;
use ribbit::atomic::A32;
use ribbit::u24;
use ribbit::u3;

use crate::node::Slot;
use crate::Node;

#[repr(C)]
pub(crate) struct Node3 {
    header: A32<Header>,

    _pad: [u32; 3],

    slots: [A128<Slot>; 3],
}

const _: () = assert!(core::mem::size_of::<Node3>() == 64);

impl Node for Node3 {
    fn get(&self, key: u8) -> Option<&A128<Slot>> {
        let header = self.header.load(Ordering::Relaxed);
        let keys = header.keys().value();
        let valid = header.valid().value();

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

    fn get_or_reserve(&self, key: u8) -> Result<&A128<Slot>, ()> {
        let old = self.header.load(Ordering::Relaxed);
        let valid = old.valid().value();
        let keys = old.keys().value();

        let mut index = 0;
        for i in 0..3 {
            // Find first invalid index
            if valid & (1u8 << i) != 1 {
                index = i;
                break;
            }

            if (keys >> (i * 8)) as u8 != key {
                continue;
            }

            return Ok(&self.slots[i]);
        }

        let keys = keys | ((key as u32) << (index * 8));
        let new = old.with_keys(u24::new(keys));

        match self
            .header
            .compare_exchange(old, new, Ordering::AcqRel, Ordering::Relaxed)
        {
            Ok(_) => Ok(&self.slots[index]),
            Err(_conflict) => todo!(),
        }
    }
}

#[ribbit::pack(size = 32)]
struct Header {
    valid: u3,
    freeze: bool,
    grow: bool,
    #[ribbit(offset = 8)]
    keys: u24,
}
