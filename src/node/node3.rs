use core::fmt::Debug;
use core::sync::atomic::Ordering;

use ribbit::atomic::A128;
use ribbit::atomic::A32;
use ribbit::u2;
use ribbit::u24;
use ribbit::u48;
use ribbit::unpack;

use crate::node;
use crate::node::FreezeError;
use crate::node::GetOrReserveError;
use crate::node::Slot;
use crate::Node;

use super::Node256;

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

    fn get_or_reserve(&self, key: u8) -> Result<&A128<Slot>, GetOrReserveError> {
        let mut old = self.header.load(Ordering::Relaxed);
        loop {
            let Some((new, index)) = old.get_or_reserve(key) else {
                return Err(GetOrReserveError::Grow);
            };

            match self
                .header
                .compare_exchange(old, new, Ordering::AcqRel, Ordering::Relaxed)
            {
                Ok(_) => return Ok(&self.slots[index as usize]),
                Err(header) if header.freeze() => {
                    return Err(GetOrReserveError::Freeze {
                        grow: header.grow(),
                    })
                }
                Err(header) => old = header,
            }
        }
    }

    fn reserve(&mut self, key: u8) -> Result<&mut A128<Slot>, GetOrReserveError> {
        // FIXME: shouldn't need atomics with &mut
        let header = self.header.load(Ordering::Relaxed);
        let (header, index) = header.get_or_reserve(key).unwrap();
        self.header.store(header, Ordering::Relaxed);
        Ok(&mut self.slots[index as usize])
    }

    fn freeze(&self, grow: bool) {
        let mut old = self.header.load(Ordering::Relaxed);

        while !old.freeze() {
            match self.header.compare_exchange(
                old,
                old.with_freeze(true).with_grow(grow),
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(conflict) => old = conflict,
            }
        }

        let grow = old.grow();
        for slot in self.slots.iter().take(old.len().value() as usize) {
            Slot::freeze(slot, grow)
        }
    }

    fn replace(&self, snapshot: &Slot) -> Slot {
        let header = self.header.load(Ordering::Relaxed);
        let keys = header.keys().value();

        assert!(header.freeze());

        let mut slots: [(u8, Slot); 3] = core::array::from_fn(|_| (0, Slot::default()));
        let mut len = 0;

        self.slots
            .iter()
            .take(header.len().value() as usize)
            .map(|slot| slot.load(Ordering::Relaxed))
            .inspect(|slot| assert!(slot.frozen()))
            .enumerate()
            .filter(|(_, slot)| {
                !matches!(
                    slot.kind().unpack(),
                    <unpack![node::Kind]>::Uninit | <unpack![node::Kind]>::Invalid
                )
            })
            .map(|(index, slot)| ((keys >> (index * 8)) as u8, slot))
            .zip(&mut slots)
            .for_each(|(slot, save)| {
                *save = slot;
                len += 1;
            });

        match len {
            // Delete
            0 => snapshot
                .with_len(0)
                .with_kind(node::Kind::new(<unpack![node::Kind]>::Uninit)),

            // Compress
            1 if snapshot.len() + 1 + slots[0].1.len() <= 8 => {
                let mut key = [0u8; 8];
                key[..snapshot.len() as usize].copy_from_slice(&snapshot.key().to_be_bytes());
                key[snapshot.len() as usize] = slots[0].0;
                key[snapshot.len() as usize..][..slots[0].1.len() as usize]
                    .copy_from_slice(&slots[0].1.key().to_be_bytes());
                let key = u64::from_be_bytes(key);

                Slot::new(
                    key,
                    snapshot.len() + 1 + slots[0].1.len(),
                    false,
                    false,
                    slots[0].1.kind(),
                    slots[0].1.next(),
                )
            }

            // Grow
            3.. if header.grow() => {
                let mut node = Box::new(Node256::new());

                for (key, slot) in slots {
                    node.reserve(key).unwrap().store(slot, Ordering::Relaxed);
                }

                let node = Box::leak(node) as *mut Node256;

                snapshot
                    .with_kind(node::Kind::new(<unpack![node::Kind]>::Node256))
                    .with_next(u48::new(node as u64))
            }

            // Replace
            _ => {
                let mut node = Box::new(Node3::new());

                for (key, slot) in slots {
                    node.reserve(key).unwrap().store(slot, Ordering::Relaxed);
                }

                let node = Box::leak(node) as *mut Node3;

                snapshot
                    .with_kind(node::Kind::new(<unpack![node::Kind]>::Node3))
                    .with_next(u48::new(node as u64))
            }
        }
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
