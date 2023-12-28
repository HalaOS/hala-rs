use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{fmt::Debug, future::Future, task::Context};

use std::{
    collections::HashMap,
    hash::Hash,
    task::{Poll, Waker},
};

pub use futures::FutureExt;
pub use shared;
use shared::{AsyncShared, AsyncSharedGuardMut};

/// Shared raw data between futures.
pub struct Condvar<E> {
    wakers: HashMap<E, (Waker, Arc<AtomicBool>)>,
}

impl<E> Default for Condvar<E> {
    fn default() -> Self {
        Self {
            wakers: Default::default(),
        }
    }
}

impl<E> Condvar<E> {
    fn on(&mut self, event: E, waker: Waker, is_on: Arc<AtomicBool>)
    where
        E: Eq + Hash + Debug,
    {
        self.wakers.insert(event, (waker, is_on));
    }

    /// Emit once `event` on
    pub fn notify(&mut self, event: E)
    where
        E: Eq + Hash + Debug,
    {
        if let Some((waker, is_on)) = self.wakers.remove(&event) {
            log::trace!("notify event={:?}, wakeup=true", event);
            is_on.store(true, Ordering::SeqCst);
            waker.wake();
        } else {
            log::trace!("notify event={:?}, wakeup=false", event);
        }
    }

    /// Emit all `events` on
    pub fn notify_all<Events: AsRef<[E]>>(&mut self, events: Events)
    where
        E: Eq + Hash + Debug + Clone,
    {
        for event in events.as_ref() {
            self.notify(event.clone());
        }
    }

    /// Notify all registered event listeners
    pub fn notify_any(&mut self) {
        for (_, (waker, is_on)) in self.wakers.drain() {
            is_on.store(true, Ordering::SeqCst);
            waker.wake();
        }
    }
}

/// A mediator is a central hub for communication between futures.
pub struct Mediator<Raw> {
    raw: Raw,
    trace_id: Option<&'static str>,
}

impl<Raw> Mediator<Raw> {}

impl<Raw> Default for Mediator<Raw>
where
    Raw: Default,
{
    fn default() -> Self {
        Self {
            raw: Default::default(),
            trace_id: None,
        }
    }
}

impl<Raw> Clone for Mediator<Raw>
where
    Raw: Clone,
{
    fn clone(&self) -> Self {
        Self {
            raw: self.raw.clone(),
            trace_id: self.trace_id.clone(),
        }
    }
}

impl<E, Raw> Mediator<Raw>
where
    E: Eq + Clone + Unpin + Hash + Debug,
    Raw: shared::AsyncShared<Value = Condvar<E>> + Unpin + Clone,
{
    /// Create new mediator with shared value.
    pub fn new() -> Self
    where
        Raw: From<Condvar<E>>,
    {
        Self {
            raw: Condvar::default().into(),
            trace_id: None,
        }
    }

    /// Create new mediator instance with `trace_id`
    pub fn new_with(trace_id: &'static str) -> Self
    where
        Raw: From<Condvar<E>>,
    {
        Self {
            raw: Condvar::default().into(),
            trace_id: Some(trace_id),
        }
    }

    /// Emit one event on.
    #[inline]
    pub fn sync_notify(&self, event: E)
    where
        E: Eq + Hash + Debug,
    {
        let mut raw = self.raw.lock_mut();

        raw.notify(event);
    }

    pub async fn notify(&self, event: E)
    where
        E: Eq + Hash + Debug,
    {
        let mut raw = self.raw.lock_mut_wait().await;

        raw.notify(event);
    }

    /// Emit all events
    #[inline]
    pub fn sync_notify_all<Events: AsRef<[E]>>(&self, events: Events) {
        let mut raw = self.raw.lock_mut();

        for event in events.as_ref() {
            raw.notify(event.clone());
        }
    }

    #[inline]
    pub async fn notify_all<Events: AsRef<[E]>>(&self, events: Events) {
        let mut raw = self.raw.lock_mut_wait().await;

        raw.notify_all(events);
    }

    #[inline]
    pub fn sync_notify_any(&self) {
        let mut raw = self.raw.lock_mut();

        raw.notify_any();
    }

    #[inline]
    pub async fn notify_any(&self) {
        let mut raw = self.raw.lock_mut_wait().await;

        raw.notify_any();
    }
    #[inline]
    pub fn event_wait<'a, T>(
        &'a self,
        lock_guard: AsyncSharedGuardMut<'a, T>,
        event: E,
    ) -> EventWait<'a, Raw, E, T>
    where
        E: Unpin + Eq + Hash + Debug,
        T: AsyncShared + Unpin,
    {
        EventWait {
            lock_guard: Some(lock_guard),
            event: Some(event),
            mediator: self,
            is_on: Arc::new(AtomicBool::new(false)),
        }
    }
}

pub struct EventWait<'a, Raw, E, T>
where
    Raw: shared::AsyncShared<Value = Condvar<E>> + Unpin + Clone,

    E: Debug + Unpin,
    T: AsyncShared + Unpin,
{
    lock_guard: Option<AsyncSharedGuardMut<'a, T>>,
    event: Option<E>,
    mediator: &'a Mediator<Raw>,
    is_on: Arc<AtomicBool>,
}

impl<'a, Raw, E, T> Future for EventWait<'a, Raw, E, T>
where
    Raw: shared::AsyncShared<Value = Condvar<E>> + Unpin + Clone,
    E: Debug + 'static + Unpin + Clone + Eq + Hash,
    T: AsyncShared + Unpin + 'static,
{
    type Output = AsyncSharedGuardMut<'a, T>;
    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // first call this poll function .
        if let Some(event) = self.event.take() {
            match Box::pin(self.mediator.raw.lock_mut_wait()).poll_unpin(cx) {
                Poll::Ready(mut raw) => {
                    self.lock_guard.as_mut().unwrap().unlock();

                    raw.on(event, cx.waker().clone(), self.is_on.clone());
                }
                _ => {
                    self.event = Some(event);
                }
            }

            return Poll::Pending;
        }

        if !self.is_on.load(Ordering::SeqCst) {
            return Poll::Pending;
        }

        {
            match Box::pin(self.lock_guard.as_mut().unwrap().relock()).poll_unpin(cx) {
                Poll::Pending => {
                    return Poll::Pending;
                }
                _ => {}
            }
        }

        Poll::Ready(self.lock_guard.take().unwrap())
    }
}

pub type LocalMediator<E> = Mediator<shared::AsyncLocalShared<Condvar<E>>>;

pub type MutexMediator<E> = Mediator<shared::AsyncMutexShared<Condvar<E>>>;

#[cfg(test)]
mod tests {
    use futures::{
        executor::{LocalPool, ThreadPool},
        task::{LocalSpawnExt, SpawnExt},
    };
    use shared::{AsyncLocalShared, AsyncMutexShared, AsyncShared};

    use crate::{LocalMediator, MutexMediator};

    #[test]
    fn test_local_mediator() {
        let mut local_pool = LocalPool::new();

        let mediator = LocalMediator::<i32>::new();

        let mediator_cloned = mediator.clone();

        let shared = AsyncLocalShared::new(1);

        let shared_cloned = shared.clone();

        local_pool
            .spawner()
            .spawn_local(async move {
                let mut shared = shared_cloned.lock_mut_wait().await;

                *shared = 2;

                mediator_cloned.notify(1).await;
            })
            .unwrap();

        local_pool.run_until(async move {
            let mut shared = shared.lock_mut_wait().await;
            if *shared != 2 {
                shared = mediator.event_wait(shared, 1).await;
            }

            assert_eq!(*shared, 2);
        });
    }

    #[futures_test::test]
    async fn test_mutex_mediator() {
        // pretty_env_logger::init_timed();

        let local_pool = ThreadPool::builder().pool_size(10).create().unwrap();

        let mediator = MutexMediator::<i32>::new();

        for _ in 0..100 {
            let mediator_cloned = mediator.clone();

            let shared = AsyncMutexShared::new(1);

            let shared_cloned = shared.clone();

            local_pool
                .spawn(async move {
                    let mut shared = shared_cloned.lock_mut_wait().await;

                    *shared = 2;

                    mediator_cloned.notify(1).await;
                })
                .unwrap();

            let mut shared = shared.lock_mut_wait().await;
            if *shared != 2 {
                shared = mediator.event_wait(shared, 1).await;
            }

            assert_eq!(*shared, 2);
        }
    }
}
