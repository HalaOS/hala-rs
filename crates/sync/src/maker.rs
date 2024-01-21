use std::{
    collections::HashMap,
    ops::{self, DerefMut},
    task::Waker,
};

#[cfg(feature = "trace_lock")]
use std::{fmt::Debug, panic::Location};

use super::*;

/// Type factory for [`AsyncLockable`]
pub struct AsyncLockableMaker<Locker, Wakers> {
    inner_locker: Locker,
    wakers: Wakers,
}

impl<Locker, Wakers> Default for AsyncLockableMaker<Locker, Wakers>
where
    Locker: Default,
    Wakers: Default,
{
    fn default() -> Self {
        Self {
            inner_locker: Default::default(),
            wakers: Default::default(),
        }
    }
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

impl<Locker, Wakers, Mediator> AsyncLockable for AsyncLockableMaker<Locker, Wakers>
where
    Locker: Lockable + Send + Sync,
    for<'a> Locker::GuardMut<'a>: Send + Unpin,
    Wakers: Lockable + Send + Sync,
    for<'b> Wakers::GuardMut<'b>: DerefMut<Target = Mediator>,
    Mediator: AsyncLockableMediator + 'static,
{
    type GuardMut<'a>= AsyncLockableMakerGuard<'a, Locker, Wakers,Mediator>
    where
        Self: 'a;

    type GuardMutFuture<'a> = AsyncLockableMakerFuture<'a, Locker, Wakers,Mediator>
    where
        Self: 'a;

    #[track_caller]
    fn lock(&self) -> Self::GuardMutFuture<'_> {
        AsyncLockableMakerFuture {
            locker: self,
            wait_key: None,
            #[cfg(feature = "trace_lock")]
            caller: Location::caller(),
        }
    }

    fn unlock<'a>(guard: Self::GuardMut<'a>) -> &'a Self {
        let locker = guard.locker;

        drop(guard);

        locker
    }
}

/// RAII `Guard` type for [`AsyncLockableMaker`]
pub struct AsyncLockableMakerGuard<'a, Locker, Wakers, Mediator>
where
    Locker: Lockable,
    Wakers: Lockable,
    for<'b> Wakers::GuardMut<'b>: DerefMut<Target = Mediator>,
    Mediator: AsyncLockableMediator,
{
    locker: &'a AsyncLockableMaker<Locker, Wakers>,
    inner_guard: Option<Locker::GuardMut<'a>>,
}

impl<'a, Locker, Wakers, Mediator> AsyncGuardMut<'a>
    for AsyncLockableMakerGuard<'a, Locker, Wakers, Mediator>
where
    Locker: Lockable + Send + Sync,
    for<'b> Locker::GuardMut<'b>: Send + Unpin,
    Wakers: Lockable + Send + Sync,
    for<'b> Wakers::GuardMut<'b>: DerefMut<Target = Mediator>,
    Mediator: AsyncLockableMediator + 'static,
{
    type Locker = AsyncLockableMaker<Locker, Wakers>;
}

impl<'a, Locker, Wakers, Mediator, T> ops::Deref
    for AsyncLockableMakerGuard<'a, Locker, Wakers, Mediator>
where
    Locker: Lockable,
    for<'c> Locker::GuardMut<'c>: ops::Deref<Target = T>,
    Wakers: Lockable,
    for<'b> Wakers::GuardMut<'b>: DerefMut<Target = Mediator>,
    Mediator: AsyncLockableMediator,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner_guard.as_deref().unwrap()
    }
}

impl<'a, Locker, Wakers, Mediator, T> ops::DerefMut
    for AsyncLockableMakerGuard<'a, Locker, Wakers, Mediator>
where
    Locker: Lockable,
    for<'c> Locker::GuardMut<'c>: ops::DerefMut<Target = T>,
    Wakers: Lockable,
    for<'b> Wakers::GuardMut<'b>: DerefMut<Target = Mediator>,
    Mediator: AsyncLockableMediator,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner_guard.as_deref_mut().unwrap()
    }
}

impl<'a, Locker, Wakers, Mediator> Drop for AsyncLockableMakerGuard<'a, Locker, Wakers, Mediator>
where
    Locker: Lockable,
    Wakers: Lockable,
    for<'b> Wakers::GuardMut<'b>: DerefMut<Target = Mediator>,
    Mediator: AsyncLockableMediator,
{
    fn drop(&mut self) {
        if let Some(guard) = self.inner_guard.take() {
            drop(guard);

            let mut wakers = self.locker.wakers.lock();

            wakers.notify_all();
        }
    }
}

/// Future created by [`lock`](AsyncLockableMaker::lock) function.
pub struct AsyncLockableMakerFuture<'a, Locker, Wakers, Mediator>
where
    Locker: Lockable,
    Wakers: Lockable,
    for<'b> Wakers::GuardMut<'b>: DerefMut<Target = Mediator>,
    Mediator: AsyncLockableMediator,
{
    locker: &'a AsyncLockableMaker<Locker, Wakers>,
    wait_key: Option<usize>,
    #[cfg(feature = "trace_lock")]
    caller: &'static Location<'static>,
}

#[cfg(feature = "trace_lock")]
impl<'a, Locker, Wakers, Mediator> Debug for AsyncLockableMakerFuture<'a, Locker, Wakers, Mediator>
where
    Locker: Lockable,
    Wakers: Lockable,
    for<'b> Wakers::GuardMut<'b>: DerefMut<Target = Mediator>,
    Mediator: AsyncLockableMediator,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "caller: {}({})", self.caller.file(), self.caller.line())
    }
}

impl<'a, Locker, Wakers, Mediator> std::future::Future
    for AsyncLockableMakerFuture<'a, Locker, Wakers, Mediator>
where
    Locker: Lockable,
    Wakers: Lockable,
    for<'b> Wakers::GuardMut<'b>: DerefMut<Target = Mediator>,
    Mediator: AsyncLockableMediator,
{
    type Output = AsyncLockableMakerGuard<'a, Locker, Wakers, Mediator>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        #[cfg(feature = "trace_lock")]
        log::trace!("async try lock, {:?}", self);

        if let Some(guard) = self.locker.inner_locker.try_lock() {
            #[cfg(feature = "trace_lock")]
            log::trace!("async locked, {:?}", self);

            return std::task::Poll::Ready(AsyncLockableMakerGuard {
                locker: self.locker,
                inner_guard: Some(guard),
            });
        }

        let mut wakers = self.locker.wakers.lock();
        self.wait_key = Some(wakers.wait_lockable(cx));

        // Ensure that we haven't raced `MutexGuard::drop`'s unlock path by
        // attempting to acquire the lock again
        if let Some(guard) = self.locker.inner_locker.try_lock() {
            #[cfg(feature = "trace_lock")]
            log::trace!("async locked, {:?}", self);

            wakers.cancel(self.wait_key.take().unwrap());

            return std::task::Poll::Ready(AsyncLockableMakerGuard {
                locker: self.locker,
                inner_guard: Some(guard),
            });
        }

        #[cfg(feature = "trace_lock")]
        log::trace!("async lock pending, {:?}", self);

        std::task::Poll::Pending
    }
}

pub struct DefaultAsyncLockableMediator {
    key_next: usize,
    wakers: HashMap<usize, Waker>,
}

impl Default for DefaultAsyncLockableMediator {
    fn default() -> Self {
        Self {
            key_next: 0,
            wakers: HashMap::new(),
        }
    }
}

impl AsyncLockableMediator for DefaultAsyncLockableMediator {
    fn wait_lockable(&mut self, cx: &mut std::task::Context<'_>) -> usize {
        let key = self.key_next;
        self.key_next += 1;

        self.wakers.insert(key, cx.waker().clone());

        key
    }

    fn cancel(&mut self, key: usize) -> bool {
        self.wakers.remove(&key).is_some()
    }

    fn notify_one(&mut self) {
        if let Some(key) = self.wakers.keys().next().map(|key| *key) {
            self.wakers.remove(&key).unwrap().wake();
        }
    }

    fn notify_all(&mut self) {
        for (_, waker) in self.wakers.drain() {
            waker.wake();
        }
    }
}
