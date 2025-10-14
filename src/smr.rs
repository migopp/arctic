#[cfg(feature = "smr-hazard")]
mod membarrier;

#[cfg(feature = "smr-hazard")]
mod hazard;

#[cfg(feature = "smr-hazard")]
pub(crate) use hazard::{Global, Guard, Local};

#[cfg(not(feature = "smr-hazard"))]
mod no_op;

#[cfg(not(feature = "smr-hazard"))]
pub(crate) use no_op::{Global, Local, ReadGuard, WriteGuard};
