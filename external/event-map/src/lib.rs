use std::{
    fmt::Debug,
    future::Future,
    hash::Hash,
    pin::Pin,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
    task::{Poll, Waker},
};

use dashmap::DashMap;
use shared::{AsyncShared, AsyncSharedGuardMut};

pub use shared;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EventMapError {
    #[error("Waiting operation canceled by user")]
    Cancel,
    #[error("Waiting operation canceled by EventMap to drop `EventMap` self")]
    Destroy,
}

#[derive(Debug, Clone, Copy)]
/// waiter wakeup reason.
pub enum Reason {
    /// Wakeup reason is unset.
    None,
    /// Waiting event on
    On,
    /// Cancel by user.
    Cancel,
    /// EventMap is dropping.
    Destroy,
}

impl From<Reason> for u8 {
    fn from(value: Reason) -> Self {
        match value {
            Reason::None => 0,
            Reason::On => 1,
            Reason::Cancel => 2,
            Reason::Destroy => 3,
        }
    }
}

#[derive(Debug, Clone)]
struct WakerWrapper {
    inner: Waker,
    /// Wakup reason,1 for success, 2 for dropping, 3 for cancel.
    reason: Arc<AtomicU8>,
}

impl WakerWrapper {
    fn wake(self, reason: Reason) {
        self.reason.store(reason.into(), Ordering::SeqCst);
        self.inner.wake();
    }

    fn wake_by_ref(&self, reason: Reason) {
        self.reason.store(reason.into(), Ordering::SeqCst);
        self.inner.wake_by_ref();
    }
}

/// Event waitable map using [`DashMap`](dashmap::DashMap) inner
#[derive(Clone, Debug, Default)]
pub struct EventMap<E>
where
    E: Send + Eq + Hash,
{
    wakers: Arc<DashMap<E, WakerWrapper>>,
}

impl<E> EventMap<E>
where
    E: Send + Eq + Hash,
{
    /// Notify one event `E` on.
    pub fn notify_one(&self, event: &E, reason: Reason) -> bool
    where
        E: Debug,
    {
        if let Some((_, waker)) = self.wakers.remove(event) {
            log::trace!("{:?} wakeup", event);
            waker.wake(reason);
            true
        } else {
            false
        }
    }

    /// Notify all event on in the providing `events` list
    pub fn notify_all<L: AsRef<[E]>>(&self, events: L, reason: Reason)
    where
        E: Debug,
    {
        for event in events.as_ref() {
            self.notify_one(event, reason);
        }
    }

    pub fn wait<'a, T>(&self, event: E, guard: AsyncSharedGuardMut<'a, T>) -> Wait<'a, T, E>
    where
        T: AsyncShared,
        E: Clone,
    {
        Wait {
            wakers: self.wakers.clone(),
            reason: Arc::new(AtomicU8::new(Reason::None.into())),
            guard: Some(guard),
            event_debug: event.clone(),
            event: Some(event),
        }
    }
}

impl<E> Drop for EventMap<E>
where
    E: Send + Eq + Hash,
{
    fn drop(&mut self) {
        if Arc::strong_count(&self.wakers) == 1 {
            // wakeup all pending future.
            for entry in self.wakers.iter() {
                entry.value().wake_by_ref(Reason::Destroy);
            }
        }
    }
}

/// Future created by [`wait`](EventMap::wait) function.
pub struct Wait<'a, T, E>
where
    E: Send + Eq + Hash,
    T: AsyncShared,
{
    wakers: Arc<DashMap<E, WakerWrapper>>,
    reason: Arc<AtomicU8>,
    event: Option<E>,
    event_debug: E,
    guard: Option<AsyncSharedGuardMut<'a, T>>,
}

impl<'a, T, E> Debug for Wait<'a, T, E>
where
    E: Send + Eq + Hash,
    T: AsyncShared,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Wait_Future({:?})", self.reason)
    }
}

impl<'a, T, E> Future for Wait<'a, T, E>
where
    E: Send + Eq + Hash + Unpin + Debug,
    T: AsyncShared + 'static + Unpin,
{
    type Output = Result<AsyncSharedGuardMut<'a, T>, EventMapError>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        if let Some(event) = self.event.take() {
            // insert waker into waiting map.
            self.wakers.insert(
                event,
                WakerWrapper {
                    inner: cx.waker().clone(),
                    reason: self.reason.clone(),
                },
            );

            // unlock guard and waiting event on.
            self.guard.as_mut().unwrap().unlock();

            log::trace!("{:?} pending", self.event_debug);

            return Poll::Pending;
        }

        // Check reason to avoid unexpected `poll` calling.
        // For example, calling `wait` function in `futures::select!` block

        let reason = self.reason.load(Ordering::SeqCst);
        if reason == Reason::None.into() {
            return Poll::Pending;
        } else if reason == Reason::Cancel.into() {
            return Poll::Ready(Err(EventMapError::Cancel));
        } else if reason == Reason::Destroy.into() {
            return Poll::Ready(Err(EventMapError::Destroy));
        }

        {
            log::trace!("acquire locker {:?}", self.event_debug);
            let mut relock = Box::pin(self.guard.as_mut().unwrap().relock());

            match Pin::new(&mut relock).poll(cx) {
                Poll::Pending => {
                    return Poll::Pending;
                }
                _ => {}
            }
        }

        Poll::Ready(Ok(self.guard.take().unwrap()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use futures::{
        executor::{LocalPool, ThreadPool},
        task::{LocalSpawnExt, SpawnExt},
    };
    use shared::{AsyncLocalShared, AsyncMutexShared};

    #[test]
    fn test_local_mediator() {
        let mut local_pool = LocalPool::new();

        let mediator = EventMap::<i32>::default();

        let shared = AsyncLocalShared::new(1);

        for i in 0..100000 {
            let shared_cloned = shared.clone();

            let mediator_cloned = mediator.clone();

            local_pool
                .spawner()
                .spawn_local(async move {
                    let mut shared = shared_cloned.lock_mut_wait().await;

                    *shared = i + 1;

                    mediator_cloned.notify_one(&i, Reason::On);
                })
                .unwrap();

            let shared_cloned = shared.clone();
            let mediator_cloned = mediator.clone();

            local_pool.run_until(async move {
                let mut shared = shared_cloned.lock_mut_wait().await;
                if *shared != i + 1 {
                    shared = mediator_cloned.wait(i, shared).await.unwrap();
                }

                assert_eq!(*shared, i + 1);
            });
        }
    }

    #[futures_test::test]
    async fn test_multi_thread_notify() {
        // pretty_env_logger::init_timed();

        let local_pool = ThreadPool::builder().pool_size(10).create().unwrap();

        let mediator = EventMap::<i32>::default();

        let shared = AsyncMutexShared::new(0);

        for i in 0..100000 {
            log::trace!("loop {} start", i);
            let mediator_cloned = mediator.clone();

            let shared_cloned = shared.clone();

            log::trace!("spwan {}", i);

            local_pool
                .spawn(async move {
                    log::trace!("spwan lock shared {}", i);
                    let mut shared = shared_cloned.lock_mut_wait().await;

                    log::trace!("spwan lock shared {} -- success", i);

                    *shared = i + 1;

                    mediator_cloned.notify_one(&i, Reason::On);
                })
                .unwrap();

            log::trace!("lock shared {}", i);
            let mut shared = shared.lock_mut_wait().await;
            log::trace!("lock shared {} -- success", i);
            if *shared != i + 1 {
                shared = mediator.wait(i, shared).await.unwrap();
            }

            assert_eq!(*shared, i + 1);

            log::trace!("loop {}", i);
        }
    }

    #[futures_test::test]
    async fn test_cancel() {
        // pretty_env_logger::init_timed();

        let local_pool = ThreadPool::builder().pool_size(10).create().unwrap();

        let mediator = EventMap::<i32>::default();

        let shared = AsyncMutexShared::new(0);

        for i in 0..100000 {
            log::trace!("loop {} start", i);
            let mediator_cloned = mediator.clone();

            let shared_cloned = shared.clone();

            log::trace!("spwan {}", i);

            local_pool
                .spawn(async move {
                    log::trace!("spwan lock shared {}", i);
                    let mut shared = shared_cloned.lock_mut_wait().await;

                    log::trace!("spwan lock shared {} -- success", i);

                    *shared = i + 1;

                    mediator_cloned.notify_one(&i, Reason::Cancel);
                })
                .unwrap();

            log::trace!("lock shared {}", i);
            let shared = shared.lock_mut_wait().await;
            log::trace!("lock shared {} -- success", i);
            if *shared != i + 1 {
                let error = mediator.wait(i, shared).await.expect_err("expect cancel");

                assert_eq!(error, EventMapError::Cancel);
            }
        }
    }
}
