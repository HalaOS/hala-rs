use std::{
    future::Future,
    ops,
    task::{Context, Poll},
};

use crate::{Locker, LockerGuard};

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

    /// Wakeup another one pending [`LockFuture`], which created by [`async_lock`](WaitableLocker::async_lock) function
    fn wakeup_another_one(&self);
}

/// `LockerGuard` for asynchronous mode
pub trait WaitableLockerGuard<'a, T: ?Sized + 'a>: LockerGuard<'a, T> {
    type Locker: WaitableLocker<WaitableGuard<'a> = Self>
    where
        Self: 'a;

    /// Get locker reference.
    fn locker(&self) -> &'a Self::Locker;

    /// [`lock`](Locker::try_lock) for asynchronous mode
    fn async_relock(&self) -> LockFuture<'a, Self::Locker>
    where
        Self: Sized,
    {
        LockFuture {
            locker: self.locker(),
        }
    }
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
