use ribbit::atomic::A128;

use crate::node::Slot;
use crate::Node;

#[repr(C)]
pub(crate) struct Node256([A128<Slot>; 256]);

impl Node for Node256 {
    fn get(&self, key: u8) -> Option<&A128<Slot>> {
        Some(&self.0[key as usize])
    }

    fn get_or_reserve(&self, key: u8) -> Result<&A128<Slot>, ()> {
        Ok(&self.0[key as usize])
    }
}
