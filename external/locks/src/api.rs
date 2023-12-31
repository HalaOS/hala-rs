use std::{
    future::Future,
    ops,
    task::{Context, Poll},
};

pub trait Locker {
    type Data: ?Sized;

    /// RAII lock guard type .
    type Guard<'a>: LockerGuard<'a, Self::Data> + ops::DerefMut<Target = Self::Data>
    where
        Self: 'a,
        Self::Data: 'a;

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

/// `Locker` for asynchronous mode
pub trait WaitableLocker: Locker {
    /// Lock guard type for immutable reference.
    type WaitableGuard<'a>: WaitableLockerGuard<'a, Self::Data, Locker = Self>
        + ops::Deref<Target = Self::Data>
    where
        Self: 'a,
        Self::Data: 'a;

    /// [`lock`](Locker::try_lock) for asynchronous mode
    fn async_lock(&self) -> LockFuture<'_, Self>
    where
        Self: Sized,
    {
        LockFuture { locker: self }
    }

    fn try_lock_with_context(&self, cx: &mut Context<'_>) -> Option<Self::WaitableGuard<'_>>;
}

pub trait LockerGuard<'a, T: ?Sized + 'a> {
    /// Manual unlock this reference lockable object.
    fn unlock(&mut self);
}

/// `LockerGuard` for asynchronous mode
pub trait WaitableLockerGuard<'a, T: ?Sized + 'a>: LockerGuard<'a, T> {
    type Locker: WaitableLocker<WaitableGuard<'a> = Self>
    where
        Self: 'a;

    /// Get locker reference.
    fn locker_ref(&self) -> &'a Self::Locker;
}

/// future create by [`lock`](WaitableLocker::lock)
pub struct LockFuture<'a, L> {
    locker: &'a L,
}

impl<'a, L> LockFuture<'a, L> {
    /// Create new lock future.
    pub fn new(locker: &'a L) -> Self {
        Self { locker }
    }
}

impl<'a, L, T> Future for LockFuture<'a, L>
where
    T: 'a,
    L: WaitableLocker<Data = T>,
{
    type Output = L::WaitableGuard<'a>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        match self.locker.try_lock_with_context(cx) {
            Some(guard) => Poll::Ready(guard),
            None => Poll::Pending,
        }
    }
}
