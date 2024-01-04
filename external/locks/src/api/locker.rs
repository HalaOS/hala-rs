use std::ops;

pub trait Locker {
    type Value: ?Sized;

    /// RAII lock guard type .
    type Guard<'a>: LockerGuard<'a> + ops::DerefMut<Target = Self::Value>
    where
        Self: 'a,
        Self::Value: 'a;

    #[must_use]
    fn new(data: Self::Value) -> Self
    where
        Self::Value: Sized;

    /// Lock this object and returns a `RAII` lock guard object.
    #[must_use]
    fn sync_lock(&self) -> Self::Guard<'_>;

    /// Attempts to acquire this `lockable` object without blocking.
    /// Returns `RAII` lock guard object if the lock was successfully acquired and `None` otherwise.
    #[must_use]
    fn try_sync_lock(&self) -> Option<Self::Guard<'_>>;

    /// Checks whether the mutex is currently locked.
    #[inline]
    fn is_locked(&self) -> bool {
        match self.try_sync_lock() {
            Some(_) => true,
            None => false,
        }
    }
}

pub trait LockerGuard<'a> {
    /// Manual unlock this reference lockable object.
    fn unlock(&mut self);
}
