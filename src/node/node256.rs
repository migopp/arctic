use ribbit::atomic::A128;

use crate::node::GetOrReserveError;
use crate::node::Slot;
use crate::Node;

#[repr(C)]
#[derive(Debug)]
pub(crate) struct Node256([A128<Slot>; 256]);

impl Node256 {
    pub(crate) fn new() -> Self {
        Self(core::array::from_fn(|_| A128::new(Slot::default())))
    }
}

impl Node for Node256 {
    fn get(&self, key: u8) -> Option<&A128<Slot>> {
        Some(&self.0[key as usize])
    }

    fn get_or_reserve(&self, key: u8) -> Result<&A128<Slot>, GetOrReserveError> {
        Ok(&self.0[key as usize])
    }

    fn reserve(&mut self, key: u8) -> Result<&mut A128<Slot>, GetOrReserveError> {
        Ok(&mut self.0[key as usize])
    }

    fn freeze(&self, grow: bool) {
        for slot in &self.0 {
            Slot::freeze(slot, grow);
        }
    }

    fn replace(&self, _snapshot: &Slot) -> Slot {
        todo!()
    }
}
