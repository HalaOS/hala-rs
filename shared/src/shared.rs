/// A trait represents a shared value that can be dynamically checked for borrowing rules
/// or protected by mutual exclusion.
pub trait Shared {
    /// The real type of this shared value.
    type Value;

    /// The immutable reference type of this shared value.
    type Ref<'a>: ops::Deref<Target = Self::Value> + Unpin
    where
        Self: 'a;

    /// The mutable reference type of this shared value.
    type RefMut<'a>: ops::DerefMut<Target = Self::Value> + Unpin
    where
        Self: 'a;

    /// Lock shared value and get immutable reference.
    fn lock(&self) -> Self::Ref<'_>;

    /// Lock shared value and get mutable reference.
    fn lock_mut(&self) -> Self::RefMut<'_>;

    /// Try lock shared value and get mutable reference.
    ///
    /// If the lock is not successful, returns [`None`]
    fn try_lock_mut(&self) -> Option<Self::RefMut<'_>>;

    /// Try lock shared value and get mutable reference.
    ///
    /// If the lock is not successful, returns [`None`]
    fn try_lock(&self) -> Option<Self::Ref<'_>>;
}

mod local;
use std::ops;

pub use local::*;

mod mutex;
pub use mutex::*;
