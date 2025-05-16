use core::sync::atomic::Ordering;

use ribbit::atomic::A128;
use ribbit::atomic::A32;
use ribbit::u24;
use ribbit::u3;

use crate::node;
use crate::node::GrowError;
use crate::node::ReserveError;
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

    fn get_or_reserve(&self, key: u8) -> Result<&A128<Slot>, ReserveError> {
        let mut old = self.header.load(Ordering::Relaxed);
        loop {
            let index = match old.get(key) {
                Ok(index) => return Ok(&self.slots[index]),
                Err(None) => return Err(ReserveError::Grow),
                Err(Some(index)) => index,
            };

            let keys = old.keys().value() | ((key as u32) << (index * 8));
            let new = old.with_keys(u24::new(keys));

            match self
                .header
                .compare_exchange(old, new, Ordering::AcqRel, Ordering::Relaxed)
            {
                Ok(_) => return Ok(&self.slots[index]),
                Err(header) if header.freeze() => {
                    return Err(ReserveError::Freeze {
                        grow: header.grow(),
                    })
                }
                Err(header) => old = header,
            }
        }
    }

    fn grow(&self, parent: &A128<Slot>) -> Result<node::Ref, GrowError> {
        let mut old = self.header.load(Ordering::Relaxed);

        if !old.freeze() {
            match self.header.compare_exchange(
                old,
                old.with_freeze(true).with_grow(true),
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => (),
                Err(header) => {
                    assert!(header.freeze());
                    old = header;
                }
            }
        }

        for slot in &self.slots {
            let old = slot.load(Ordering::Relaxed);

            if old.freeze() {
                continue;
            }

            // Safe to ignore result here
            let _ = slot.compare_exchange(
                old,
                old.with_freeze(true),
                Ordering::AcqRel,
                Ordering::Relaxed,
            );
        }

        for (i, slot) in self.slots.iter().enumerate() {
            let slot = slot.load(Ordering::Relaxed);
        }

        todo!()
    }

    fn help(&self, parent: &A128<Slot>, grow: bool) -> Result<(), ()> {
        todo!()
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

impl Header {
    /// Return `Ok(index)` if the key is mapped, `Err(Some(index))`
    /// if there is an available slot, or `Err(None)` otherwise.
    fn get(&self, key: u8) -> Result<usize, Option<usize>> {
        let valid = self.valid().value();
        let keys = self.keys().value();

        for i in 0..3 {
            if valid & (1u8 << i) != 1 {
                return Err(Some(i));
            }

            if (keys >> (i * 8)) as u8 == key {
                return Ok(i);
            }
        }

        Err(None)
    }
}
