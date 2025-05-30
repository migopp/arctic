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
        let index = self.header.load(Ordering::Acquire).get(key)?;
        Some(&self.slots[index as usize])
    }

    fn get_or_reserve(&self, key: u8) -> Result<&A128<Slot>, ReserveError> {
        let mut old = self.header.load(Ordering::Relaxed);
        loop {
            let (new, index) = old.get_or_reserve(key).unwrap();

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

    fn reserve(&mut self, key: u8) -> Result<&mut A128<Slot>, ReserveError> {
        // FIXME: shouldn't need atomics with &mut
        let header = self.header.load(Ordering::Relaxed);
        let (header, index) = header.get_or_reserve(key).unwrap();
        self.header.store(header, Ordering::Relaxed);
        Ok(&mut self.slots[index as usize])
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
    fn get(&self, key: u8) -> Option<u8> {
        let keys = self.keys().value();
        let len = self.len().value();
        (0..len).find(|i| (keys >> (i * 8)) as u8 == key)
    }

    fn get_or_reserve(&self, key: u8) -> Option<(Self, u8)> {
        if let Some(index) = self.get(key) {
            return Some((*self, index));
        }

        let keys = self.keys().value();
        let len = self.len().value();
        match len {
            0..3 => Some((
                self.with_len(u2::new(len + 1))
                    .with_freeze(false)
                    .with_keys(u24::new(keys | ((key as u32) << (len * 8)))),
                len,
            )),
            _ => None,
        }
    }
}
