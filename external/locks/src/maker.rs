use std::{
    collections::VecDeque,
    ops::{self, DerefMut},
    task::Waker,
};

use super::*;

/// Type factory for [`AsyncLockable`]
pub struct AsyncLockableMaker<Locker, Wakers> {
    inner_locker: Locker,
    wakers: Wakers,
}

impl<Locker, Wakers> AsyncLockableMaker<Locker, Wakers>
where
    Locker: LockableNew,
    Wakers: Default,
{
    pub fn new(value: Locker::Value) -> Self {
        Self {
            inner_locker: Locker::new(value),
            wakers: Default::default(),
        }
    }
}

impl<Locker, Wakers> AsyncLockable for AsyncLockableMaker<Locker, Wakers>
where
    Locker: Lockable + Send + Sync,
    for<'a> Locker::GuardMut<'a>: Send,
    Wakers: Lockable + Send + Sync,
    for<'b> Wakers::GuardMut<'b>: DerefMut<Target = VecDeque<Waker>>,
{
    type GuardMut<'a>= AsyncLockableMakerGuard<'a, Locker, Wakers>
    where
        Self: 'a;

    type GuardMutFuture<'a> = AsyncLockableMakerFuture<'a, Locker, Wakers>
    where
        Self: 'a;

    fn lock(&self) -> Self::GuardMutFuture<'_> {
        AsyncLockableMakerFuture { locker: self }
    }

    fn unlock(guard: Self::GuardMut<'_>) -> &Self {
        let locker = guard.locker;

        drop(guard);

        locker
    }
}

/// RAII `Guard` type for [`AsyncLockableMaker`]
pub struct AsyncLockableMakerGuard<'a, Locker, Wakers>
where
    Locker: Lockable,
    Wakers: Lockable,
    for<'b> Wakers::GuardMut<'b>: DerefMut<Target = VecDeque<Waker>>,
{
    locker: &'a AsyncLockableMaker<Locker, Wakers>,
    inner_guard: Option<Locker::GuardMut<'a>>,
}

impl<'a, Locker, Wakers, T> ops::Deref for AsyncLockableMakerGuard<'a, Locker, Wakers>
where
    Locker: Lockable,
    for<'c> Locker::GuardMut<'c>: ops::Deref<Target = T>,
    Wakers: Lockable,
    for<'b> Wakers::GuardMut<'b>: DerefMut<Target = VecDeque<Waker>>,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner_guard.as_deref().unwrap()
    }
}

impl<'a, Locker, Wakers, T> ops::DerefMut for AsyncLockableMakerGuard<'a, Locker, Wakers>
where
    Locker: Lockable,
    for<'c> Locker::GuardMut<'c>: ops::DerefMut<Target = T>,
    Wakers: Lockable,
    for<'b> Wakers::GuardMut<'b>: DerefMut<Target = VecDeque<Waker>>,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner_guard.as_deref_mut().unwrap()
    }
}

impl<'a, Locker, Wakers> Drop for AsyncLockableMakerGuard<'a, Locker, Wakers>
where
    Locker: Lockable,
    Wakers: Lockable,
    for<'b> Wakers::GuardMut<'b>: DerefMut<Target = VecDeque<Waker>>,
{
    fn drop(&mut self) {
        if let Some(guard) = self.inner_guard.take() {
            drop(guard);

            if let Some(wake) = self.locker.wakers.lock().pop_front() {
                wake.wake();
            }
        }
    }
}

/// Future created by [`lock`](AsyncLockableMaker::lock) function.
pub struct AsyncLockableMakerFuture<'a, Locker, Wakers>
where
    Locker: Lockable,
{
    locker: &'a AsyncLockableMaker<Locker, Wakers>,
}

impl<'a, Locker, Wakers> std::future::Future for AsyncLockableMakerFuture<'a, Locker, Wakers>
where
    Locker: Lockable,
    Wakers: Lockable,
    for<'b> Wakers::GuardMut<'b>: DerefMut<Target = VecDeque<Waker>>,
{
    type Output = AsyncLockableMakerGuard<'a, Locker, Wakers>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let mut wakers = self.locker.wakers.lock();

        match self.locker.inner_locker.try_lock() {
            Some(guard) => std::task::Poll::Ready(AsyncLockableMakerGuard {
                locker: self.locker,
                inner_guard: Some(guard),
            }),
            None => {
                wakers.push_back(cx.waker().clone());

                std::task::Poll::Pending
            }
        }
    }
}
