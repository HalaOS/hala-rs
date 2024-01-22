use std::{
    collections::HashMap,
    ops,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
    task::Waker,
};

#[cfg(feature = "trace_lock")]
use std::{fmt::Debug, panic::Location};

use hala_lockfree::queue::Queue;

use super::*;

const WAKER_UNINIT: u8 = 0;
const WAKER_INIT: u8 = 1;
const WAKER_DROP: u8 = 1;

/// Type factory for [`AsyncLockable`]
pub struct AsyncLockableMaker<Locker> {
    inner_locker: Locker,
    wakers: Queue<(Waker, Arc<AtomicU8>)>,
}

impl<Locker> Default for AsyncLockableMaker<Locker>
where
    Locker: Default,
{
    fn default() -> Self {
        Self {
            inner_locker: Default::default(),
            wakers: Default::default(),
        }
    }
}

impl<Locker> AsyncLockableMaker<Locker>
where
    Locker: LockableNew,
{
    pub fn new(value: Locker::Value) -> Self {
        Self {
            inner_locker: Locker::new(value),
            wakers: Default::default(),
        }
    }
}

impl<Locker> AsyncLockable for AsyncLockableMaker<Locker>
where
    Locker: Lockable + Send + Sync,
    for<'a> Locker::GuardMut<'a>: Send + Unpin,
{
    type GuardMut<'a>= AsyncLockableMakerGuard<'a, Locker>
    where
        Self: 'a;

    type GuardMutFuture<'a> = AsyncLockableMakerFuture<'a, Locker>
    where
        Self: 'a;

    #[track_caller]
    fn lock(&self) -> Self::GuardMutFuture<'_> {
        AsyncLockableMakerFuture {
            locker: self,
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
pub struct AsyncLockableMakerGuard<'a, Locker>
where
    Locker: Lockable,
{
    locker: &'a AsyncLockableMaker<Locker>,
    inner_guard: Option<Locker::GuardMut<'a>>,
}

impl<'a, Locker> AsyncGuardMut<'a> for AsyncLockableMakerGuard<'a, Locker>
where
    Locker: Lockable + Send + Sync,
    for<'b> Locker::GuardMut<'b>: Send + Unpin,
{
    type Locker = AsyncLockableMaker<Locker>;
}

impl<'a, Locker, T> ops::Deref for AsyncLockableMakerGuard<'a, Locker>
where
    Locker: Lockable,
    for<'c> Locker::GuardMut<'c>: ops::Deref<Target = T>,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner_guard.as_deref().unwrap()
    }
}

impl<'a, Locker, T> ops::DerefMut for AsyncLockableMakerGuard<'a, Locker>
where
    Locker: Lockable,
    for<'c> Locker::GuardMut<'c>: ops::DerefMut<Target = T>,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner_guard.as_deref_mut().unwrap()
    }
}

impl<'a, Locker> Drop for AsyncLockableMakerGuard<'a, Locker>
where
    Locker: Lockable,
{
    fn drop(&mut self) {
        if let Some(guard) = self.inner_guard.take() {
            drop(guard);

            while let Some((waker, flag)) = self.locker.wakers.pop() {
                match flag.load(Ordering::Acquire) {
                    WAKER_INIT => {
                        waker.wake();
                        break;
                    }
                    WAKER_UNINIT => {
                        self.locker.wakers.push((waker, flag));
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Future created by [`lock`](AsyncLockableMaker::lock) function.
pub struct AsyncLockableMakerFuture<'a, Locker>
where
    Locker: Lockable,
{
    locker: &'a AsyncLockableMaker<Locker>,
    #[cfg(feature = "trace_lock")]
    caller: &'static Location<'static>,
}

#[cfg(feature = "trace_lock")]
impl<'a, Locker> Debug for AsyncLockableMakerFuture<'a, Locker>
where
    Locker: Lockable,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "caller: {}({})", self.caller.file(), self.caller.line())
    }
}

impl<'a, Locker> std::future::Future for AsyncLockableMakerFuture<'a, Locker>
where
    Locker: Lockable,
{
    type Output = AsyncLockableMakerGuard<'a, Locker>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
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

        let flag = Arc::new(AtomicU8::new(WAKER_UNINIT));

        self.locker.wakers.push((cx.waker().clone(), flag.clone()));

        // Ensure that we haven't raced `MutexGuard::drop`'s unlock path by
        // attempting to acquire the lock again
        if let Some(guard) = self.locker.inner_locker.try_lock() {
            #[cfg(feature = "trace_lock")]
            log::trace!("async locked, {:?}", self);

            flag.store(WAKER_DROP, Ordering::Release);

            return std::task::Poll::Ready(AsyncLockableMakerGuard {
                locker: self.locker,
                inner_guard: Some(guard),
            });
        }

        #[cfg(feature = "trace_lock")]
        log::trace!("async lock pending, {:?}", self);

        flag.store(WAKER_INIT, Ordering::Release);

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
