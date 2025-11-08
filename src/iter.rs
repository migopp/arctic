#[expect(private_bounds)]
pub trait Order: crate::raw::iter::Order {}

impl<T: crate::raw::iter::Order> Order for T {}

pub use crate::raw::iter::sort::Sorted;
pub use crate::raw::iter::sort::Unsorted;

pub struct Include<T>(pub(crate) T);
pub struct Exclude<T>(pub(crate) T);
#[derive(Copy, Clone, Default)]
pub struct Unbound;
