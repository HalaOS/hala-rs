use std::{
    fmt::Debug,
    future::Future,
    ops::{Deref, DerefMut},
    sync::Arc,
    task::Context,
};

use futures::lock::{Mutex, OwnedMutexLockFuture};

use std::{
    collections::HashMap,
    hash::Hash,
    task::{Poll, Waker},
};

use futures::FutureExt;

/// Shared raw data between futures.
pub struct Shared<T, E> {
    value: T,
    wakers: HashMap<E, Waker>,
}

impl<T, E> Shared<T, E> {
    fn new(value: T) -> Self {
        Self {
            value: value.into(),
            wakers: Default::default(),
        }
    }

    fn register_event_listener(&mut self, event: E, waker: Waker)
    where
        E: Eq + Hash,
    {
        self.wakers.insert(event, waker);
    }

    /// Emit once `event` on
    pub fn notify(&mut self, event: E)
    where
        E: Eq + Hash + Debug,
    {
        if let Some(waker) = self.wakers.remove(&event) {
            log::trace!("notify event={:?}, wakeup=true", event);
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

    /// Get shared value immutable reference.
    pub fn value(&self) -> &T {
        &self.value
    }

    /// Get shared value mutable reference.
    pub fn value_mut(&mut self) -> &mut T {
        &mut self.value
    }
}

impl<T, E> Deref for Shared<T, E> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T, E> DerefMut for Shared<T, E> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

/// A mediator is a central hub for communication between futures.
#[derive(Debug)]
pub struct Mediator<T, E> {
    raw: Arc<Mutex<Shared<T, E>>>,
}

impl<T, E> Clone for Mediator<T, E> {
    fn clone(&self) -> Self {
        Self {
            raw: self.raw.clone(),
        }
    }
}

impl<T, E> Mediator<T, E> {
    /// Create new mediator with shared value.
    pub fn new(value: T) -> Self {
        Self {
            raw: Arc::new(Mutex::new(Shared::new(value))),
        }
    }

    /// Acquire the lock and access immutable shared data.
    pub async fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        let raw = self.raw.lock().await;

        f(&raw.value)
    }

    /// Acquire the lock and access mutable shared data.
    pub async fn with_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        let mut raw = self.raw.lock().await;

        f(&mut raw.value)
    }

    /// Attempt to acquire the shared data lock immediately.
    ///
    /// If the lock is currently held, this will return `None`.
    pub fn try_lock(&self) -> Option<futures::lock::MutexGuard<'_, Shared<T, E>>> {
        self.raw.try_lock()
    }

    /// Emit one event on.
    pub async fn notify(&self, event: E)
    where
        E: Eq + Hash + Debug,
    {
        let mut raw = self.raw.lock().await;

        raw.notify(event);
    }

    /// Emit all events
    pub async fn notify_all<Events: AsRef<[E]>>(&self, events: Events)
    where
        E: Eq + Hash + Clone + Debug,
    {
        let mut raw = self.raw.lock().await;

        for event in events.as_ref() {
            raw.notify(event.clone());
        }
    }

    /// Create a new event handle future with poll function `f` and run once immediately
    ///
    /// If `f` returns [`Pending`](Poll::Pending), the system will move the
    /// handle into the event waiting map, and the future returns `Pending` status.
    ///
    /// You can call [`notify`](Mediator::notify) to wake up this poll function and run once again.
    pub fn on_fn<F, R>(&self, event: E, f: F) -> OnEvent<T, E, F>
    where
        F: FnMut(&mut Shared<T, E>, &mut Context<'_>) -> Poll<R> + Unpin,
        T: Unpin + 'static,
        E: Unpin + Eq + Hash + Debug,
        R: Unpin,
    {
        OnEvent {
            f: Some(f),
            raw: self.raw.clone(),
            lock_future: None,
            event,
        }
    }
}

/// Future create by [`on`](Mediator::on)
pub struct OnEvent<T, E, F>
where
    E: Debug,
{
    f: Option<F>,
    raw: Arc<Mutex<Shared<T, E>>>,
    lock_future: Option<OwnedMutexLockFuture<Shared<T, E>>>,
    event: E,
}

impl<T, E, F, R> Future for OnEvent<T, E, F>
where
    F: FnMut(&mut Shared<T, E>, &mut Context<'_>) -> Poll<R> + Unpin,
    T: Unpin,
    E: Unpin + Eq + Hash + Copy,
    R: Unpin,
    E: Debug,
{
    type Output = R;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Self::Output> {
        let mut lock_future = if let Some(lock_future) = self.lock_future.take() {
            lock_future
        } else {
            self.raw.clone().lock_owned()
        };

        let mut raw = match lock_future.poll_unpin(cx) {
            Poll::Ready(raw) => raw,
            _ => {
                self.lock_future = Some(lock_future);

                return Poll::Pending;
            }
        };

        let mut f = self.f.take().unwrap();

        match f(&mut raw, cx) {
            Poll::Pending => {
                self.f = Some(f);

                raw.register_event_listener(self.event, cx.waker().clone());

                return Poll::Pending;
            }
            poll => {
                return poll;
            }
        }
    }
}

/// Register event handle with async fn
#[macro_export]
macro_rules! on {
    ($mediator: expr, $event: expr, $fut: expr) => {
        $mediator.on_fn(Event::A, |mediator_cx, cx| {
            use $crate::FutureExt;
            Box::pin($fut(mediator_cx)).poll_unpin(cx)
        })
    };
}

#[cfg(test)]
mod tests {
    use std::task::Poll;

    use futures::executor::ThreadPool;

    use futures::task::SpawnExt;

    use crate::{Mediator, Shared};

    #[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
    enum Event {
        A,
        B,
    }

    #[futures_test::test]
    async fn test_mediator() {
        let mediator: Mediator<i32, Event> = Mediator::new(1);

        let thread_pool = ThreadPool::builder().pool_size(10).create().unwrap();

        thread_pool
            .spawn(mediator.on_fn(Event::B, |mediator_cx, _| {
                if *mediator_cx.value() == 1 {
                    *mediator_cx.value_mut() = 2;
                    mediator_cx.notify(Event::A);

                    return Poll::Ready(());
                }

                return Poll::Pending;
            }))
            .unwrap();

        mediator
            .on_fn(Event::A, |mediator_cx, _| {
                if *mediator_cx.value() == 1 {
                    return Poll::Pending;
                }

                return Poll::Ready(());
            })
            .await;
    }

    #[futures_test::test]
    async fn test_mediator_async() {
        let mediator: Mediator<i32, Event> = Mediator::new(1);

        let thread_pool = ThreadPool::builder().pool_size(10).create().unwrap();

        async fn assign_2(cx: &mut Shared<i32, Event>) {
            *cx.value_mut() = 2;
        }

        thread_pool
            .spawn_with_handle(on!(mediator, Event::A, assign_2))
            .unwrap()
            .await;

        assert_eq!(mediator.with(|value| *value).await, 2);
    }
}
