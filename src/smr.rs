#[cfg(feature = "smr-hazard")]
mod membarrier;

#[cfg(feature = "smr-hazard")]
mod hazard;

#[cfg(feature = "smr-hazard")]
pub(crate) use hazard::{Global, Local, Owned, PathGuard, Shared};

#[cfg(not(feature = "smr-hazard"))]
mod no_op;

#[cfg(not(feature = "smr-hazard"))]
pub(crate) use no_op::{Global, Guard, Local};
