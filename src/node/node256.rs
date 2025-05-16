use crate::node::Slot;

#[repr(C)]
pub(crate) struct Node256([Slot; 256]);
