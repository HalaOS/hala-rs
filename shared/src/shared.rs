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
    fn lock(&self) -> SharedGuard<'_, Self>;

    /// Lock shared value and get mutable reference.
    fn lock_mut(&self) -> SharedGuardMut<'_, Self>;

    /// Try lock shared value and get mutable reference.
    ///
    /// If the lock is not successful, returns [`None`]
    fn try_lock_mut(&self) -> Option<SharedGuardMut<'_, Self>>;

    /// Try lock shared value and get mutable reference.
    ///
    /// If the lock is not successful, returns [`None`]
    fn try_lock(&self) -> Option<SharedGuard<'_, Self>>;
}

pub struct SharedGuard<'a, S>
where
    S: Shared + ?Sized,
{
    pub value: Option<S::Ref<'a>>,
    pub shared: &'a S,
}

impl<'a, S> ops::Deref for SharedGuard<'a, S>
where
    S: Shared,
{
    type Target = S::Value;

    fn deref(&self) -> &Self::Target {
        &self.value.as_ref().expect("unlocked")
    }
}

impl<'a, S> SharedGuard<'a, S>
where
    S: Shared + ?Sized,
{
    pub fn unlock(&mut self) {
        self.value.take();
    }

    pub fn relock(&mut self) {
        self.value = self.shared.lock().value
    }
}

pub struct SharedGuardMut<'a, S>
where
    S: Shared + ?Sized,
{
    pub value: Option<S::RefMut<'a>>,
    pub shared: &'a S,
}

impl<'a, S> ops::Deref for SharedGuardMut<'a, S>
where
    S: Shared,
{
    type Target = S::Value;

    fn deref(&self) -> &Self::Target {
        &self.value.as_ref().expect("unlocked")
    }
}

impl<'a, S> ops::DerefMut for SharedGuardMut<'a, S>
where
    S: Shared,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value.as_deref_mut().expect("unlocked")
    }
}

impl<'a, S> SharedGuardMut<'a, S>
where
    S: Shared + ?Sized,
{
    pub fn unlock(&mut self) {
        self.value.take();
    }

    pub fn relock(&mut self) {
        self.value = self.shared.lock_mut().value
    }
}

mod local;
use std::ops;

pub use local::*;

mod mutex;
pub use mutex::*;
