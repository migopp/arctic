use ribbit::atomic::A128;

use crate::node::Slot;
use crate::Node;

#[repr(C)]
pub(crate) struct Node256([A128<Slot>; 256]);

impl Node for Node256 {
    fn get(&self, key: &mut &[u8]) -> Option<&A128<Slot>> {
        let (head, tail) = key.split_first()?;
        *key = tail;
        Some(&self.0[*head as usize])
    }
}
