mod api;
pub use api::*;
/// [`AyncLockable`] type maker
pub mod maker;

#[cfg(not(feature = "use_parking_lot"))]
mod spin;
#[cfg(not(feature = "use_parking_lot"))]
pub use spin::*;

#[cfg(feature = "use_parking_lot")]
mod parking_lot;
#[cfg(feature = "use_parking_lot")]
pub use parking_lot::*;
