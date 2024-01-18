use std::future::Future;

/// Any mutex object should implement this trait
pub trait Lockable {
    /// RAII scoped lock guard type.
    type GuardMut<'a>
    where
        Self: 'a;

    /// Lock self and returns RAII locker guard object.
    fn lock(&self) -> Self::GuardMut<'_>;

    /// Attempts to acquire this lock.
    ///
    /// If the lock could not be acquired at this time, then `None` is returned.
    /// Otherwise, an RAII guard is returned. The lock will be unlocked when the guard is dropped.
    fn try_lock(&self) -> Option<Self::GuardMut<'_>>;

    /// Immediately drops the `guard`, and consequently unlocks the `Lockable` object.
    ///
    /// The return value is the associated [`Lockable`] object of this `guard`
    fn unlock(guard: Self::GuardMut<'_>) -> &Self;
}

pub trait LockableNew: Lockable {
    type Value;
    fn new(value: Self::Value) -> Self;
}

/// A futures-aware lockable mutex object should implement this trait.
pub trait AsyncLockable {
    /// RAII scoped lock guard type.
    ///
    /// A guard of futures-aware mutex must be able to transfer between threads
    /// In other words, this guard must not track any thread-specific details
    type GuardMut<'a>: AsyncGuardMut<'a, Locker = Self> + Send + Unpin
    where
        Self: 'a;

    /// Future created by [`lock`](AsyncLockable::lock) function
    type GuardMutFuture<'a>: Future<Output = Self::GuardMut<'a>> + Send
    where
        Self: 'a;

    /// Acquire the lock asynchronously.
    fn lock(&self) -> Self::GuardMutFuture<'_>;

    /// Immediately drops the `guard`, and consequently unlocks the `Lockable` object.
    ///
    /// The return value is the associated [`Lockable`] object of this `guard`
    fn unlock<'a>(guard: Self::GuardMut<'a>) -> &'a Self;
}

pub trait AsyncGuardMut<'a> {
    type Locker: AsyncLockable<GuardMut<'a> = Self>
    where
        Self: 'a;
}
