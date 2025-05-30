use core::fmt::Debug;
use core::sync::atomic::Ordering;

use ribbit::atomic::A128;
use ribbit::atomic::A32;
use ribbit::u2;
use ribbit::u24;

use crate::node;
use crate::node::GrowError;
use crate::node::ReserveError;
use crate::node::Slot;
use crate::Node;

#[repr(C)]
#[derive(Debug)]
pub(crate) struct Node3 {
    header: A32<Header>,

    _pad: [u32; 3],

    slots: [A128<Slot>; 3],
}

const _: () = assert!(core::mem::size_of::<Node3>() == 64);

impl Node3 {
    pub(crate) fn new() -> Self {
        Self {
            header: A32::new(Header::default()),
            _pad: [0; 3],
            slots: core::array::from_fn(|_| A128::new(Slot::default())),
        }
    }
}

impl Node for Node3 {
    fn get(&self, key: u8) -> Option<&A128<Slot>> {
        let index = self.header.load(Ordering::Acquire).get(key).ok()?;
        Some(&self.slots[index as usize])
    }

    fn get_or_reserve(&self, key: u8) -> Result<&A128<Slot>, ReserveError> {
        let mut old = self.header.load(Ordering::Relaxed);
        loop {
            let index = match old.get(key) {
                Ok(index) => return Ok(&self.slots[index as usize]),
                Err(None) => return Err(ReserveError::Grow),
                Err(Some(index)) => index,
            };

            let keys = old.keys().value() | ((key as u32) << (index * 8));
            let new = old.with_keys(u24::new(keys));

            match self
                .header
                .compare_exchange(old, new, Ordering::AcqRel, Ordering::Relaxed)
            {
                Ok(_) => return Ok(&self.slots[index as usize]),
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

#[ribbit::pack(size = 32, debug)]
struct Header {
    len: u2,
    freeze: bool,
    grow: bool,
    #[ribbit(offset = 8)]
    keys: u24,
}

impl Default for Header {
    fn default() -> Self {
        Self::new(u2::new(0), false, false, u24::new(0))
    }
}

impl Header {
    /// Return `Ok(index)` if the key is mapped, `Err(Some(index))`
    /// if there is an available slot, or `Err(None)` otherwise.
    fn get(&self, key: u8) -> Result<u8, Option<u8>> {
        let keys = self.keys().value();
        let len = self.len().value();

        for i in 0..len {
            if (keys >> (i * 8)) as u8 == key {
                return Ok(i);
            }
        }

        match len {
            0..3 => Err(Some(len)),
            _ => Err(None),
        }
    }
}
