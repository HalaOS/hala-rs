use std::{
    future::Future,
    task::{Context, Poll},
};

/// `Locker` for asynchronous mode
pub trait WaitableLocker {
    /// Lock guard type for immutable reference.
    type WaitableGuard<'a>: WaitableLockerGuard<'a, Locker = Self>
    where
        Self: 'a;

    /// [`lock`](Locker::try_lock) for asynchronous mode
    fn async_lock(&self) -> LockFuture<'_, Self>
    where
        Self: Sized,
    {
        LockFuture { locker: self }
    }

    fn poll_lock(&self, cx: &mut Context<'_>) -> Poll<Self::WaitableGuard<'_>>;

    /// Wakeup another one pending [`LockFuture`], which created by [`async_lock`](WaitableLocker::async_lock) function
    fn wakeup_another_one(&self);
}

/// `LockerGuard` for asynchronous mode
pub trait WaitableLockerGuard<'a> {
    type Locker: WaitableLocker<WaitableGuard<'a> = Self>
    where
        Self: 'a;
    /// Get locker reference.
    fn locker(&self) -> &'a Self::Locker;

    fn unlock(&mut self);
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

impl<'a, L> Future for LockFuture<'a, L>
where
    L: WaitableLocker,
{
    type Output = L::WaitableGuard<'a>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        self.locker.poll_lock(cx)
    }
}
