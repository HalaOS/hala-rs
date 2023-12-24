use std::{
    collections::VecDeque,
    fmt::Debug,
    future::Future,
    ops::{Deref, DerefMut},
    task::Context,
};

use std::{
    collections::HashMap,
    hash::Hash,
    task::{Poll, Waker},
};

pub use futures::FutureExt;

/// Shared raw data between futures.
pub struct SharedData<T, E> {
    value: T,
    wakers: HashMap<E, Waker>,
}

impl<T, E> Default for SharedData<T, E>
where
    T: Default,
{
    fn default() -> Self {
        Self {
            value: Default::default(),
            wakers: Default::default(),
        }
    }
}

impl<T, E> SharedData<T, E> {
    fn new(value: T) -> Self {
        Self {
            value: value.into(),
            wakers: Default::default(),
        }
    }

    fn add_listener(&mut self, event: E, waker: Waker)
    where
        E: Eq + Hash + Debug,
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

    /// Wakeup all listener.
    pub fn wakeup_all(&mut self) {
        for (_, waker) in self.wakers.drain() {
            waker.wake();
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

impl<T, E> Deref for SharedData<T, E> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T, E> DerefMut for SharedData<T, E> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

/// A mediator is a central hub for communication between futures.
pub struct Mediator<Raw, Wakers> {
    raw: Raw,
    wakers: Wakers,
    trace_id: Option<&'static str>,
}

impl<Raw, Wakers> Mediator<Raw, Wakers> {}

impl<Raw, Wakers> Default for Mediator<Raw, Wakers>
where
    Raw: Default,
    Wakers: Default,
{
    fn default() -> Self {
        Self {
            raw: Default::default(),
            wakers: Default::default(),
            trace_id: None,
        }
    }
}

impl<Raw, Wakers> Clone for Mediator<Raw, Wakers>
where
    Raw: Clone,
    Wakers: Clone,
{
    fn clone(&self) -> Self {
        Self {
            raw: self.raw.clone(),
            wakers: self.wakers.clone(),
            trace_id: self.trace_id.clone(),
        }
    }
}

impl<T, E, Raw, Wakers> Mediator<Raw, Wakers>
where
    T: Unpin + 'static,
    E: Eq + Clone + Unpin + Hash + Debug,
    Raw: shared::Shared<Value = SharedData<T, E>> + From<SharedData<T, E>> + Unpin + Clone,
    Wakers: shared::Shared<Value = VecDeque<Waker>> + From<VecDeque<Waker>> + Unpin + Clone,
{
    /// Create new mediator with shared value.
    pub fn new(value: T) -> Self {
        Self {
            raw: SharedData::new(value).into(),
            wakers: VecDeque::default().into(),
            trace_id: None,
        }
    }

    /// Create new mediator instance with `trace_id`
    pub fn new_with(value: T, trace_id: &'static str) -> Self {
        Self {
            raw: SharedData::new(value).into(),
            wakers: VecDeque::default().into(),
            trace_id: Some(trace_id),
        }
    }

    fn notify_one_lock_waiter(&self) {
        let mut wakers = self.wakers.lock_mut();

        if let Some(waker) = wakers.pop_front() {
            waker.wake();
        }
    }

    /// Acquires an immutable reference of `SharedData`.
    ///
    /// When this function returns, it will notify another locker `waiter`.
    pub fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&SharedData<T, E>) -> R,
    {
        let raw = self.raw.lock();

        let r = f(&raw);

        self.notify_one_lock_waiter();

        r
    }

    /// Acquires a mutable reference of `SharedData`.
    ///
    /// When this function returns, it will notify another locker `waiter`.
    pub fn with_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut SharedData<T, E>) -> R,
    {
        let mut raw = self.raw.lock_mut();

        let r = f(&mut raw);

        self.notify_one_lock_waiter();

        r
    }

    /// Emit one event on.
    pub fn notify(&self, event: E)
    where
        E: Eq + Hash + Debug,
    {
        self.with_mut(|shared| shared.notify(event));
    }

    /// Emit all events
    pub fn notify_all<Events: AsRef<[E]>>(&self, events: Events) {
        self.with_mut(|shared| {
            for event in events.as_ref() {
                shared.notify(event.clone());
            }
        });
    }

    /// Create a future that wraps a function that returns a Poll.
    ///
    /// When this function returns [`Pending`](Poll::Pending), `Mediator` registers this function in the `event` waitlist.
    ///
    /// You can call [`notify`](Mediator::notify) or [`notify_all`](Mediator::notify) to wakeup this poll function again.
    #[inline]
    pub fn on_poll<F, R>(&self, event: E, f: F) -> OnEvent<Raw, Wakers, E, F>
    where
        F: FnMut(&mut SharedData<T, E>, &mut Context<'_>) -> Poll<R> + Unpin,
        T: Unpin + 'static,
        E: Unpin + Eq + Hash + Debug,
        R: Unpin,
    {
        log::trace!(target:self.trace_id.unwrap_or(""), "call on_fn {:?}", event);

        OnEvent {
            f: Some(f),
            raw: self.raw.clone(),
            wakers: self.wakers.clone(),
            event,
            trace_id: self.trace_id,
        }
    }
}

/// Future create by [`on`](Mediator::on_fn)
pub struct OnEvent<Raw, Wakers, E, F>
where
    E: Debug,
{
    f: Option<F>,
    raw: Raw,
    wakers: Wakers,
    event: E,
    #[allow(unused)]
    trace_id: Option<&'static str>,
}

impl<Raw, Wakers, T, E, F, R> Future for OnEvent<Raw, Wakers, E, F>
where
    Raw: shared::Shared<Value = SharedData<T, E>> + Unpin + Clone,
    Wakers: shared::Shared<Value = VecDeque<Waker>> + Unpin + Clone,
    F: FnMut(&mut SharedData<T, E>, &mut Context<'_>) -> Poll<R> + Unpin,
    T: Unpin,
    E: Unpin + Eq + Hash + Clone,
    R: Unpin,
    E: Debug,
{
    type Output = R;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Self::Output> {
        let trace_id = self.trace_id.unwrap_or("");

        // log::trace!(target: trace_id, "poll {:?}", self.event);

        let poll = {
            let raw = self.raw.clone();

            let mut raw = match raw.try_lock_mut() {
                Some(raw) => raw,
                _ => {
                    // log::trace!(target: trace_id, "poll {:?}, lock pending", self.event);
                    self.wakers.lock_mut().push_back(cx.waker().clone());
                    return Poll::Pending;
                }
            };

            // log::trace!(target: trace_id, "poll {:?}, locked", self.event);

            let mut f = self.f.take().unwrap();

            match f(&mut raw, cx) {
                Poll::Pending => {
                    self.f = Some(f);

                    log::trace!(target: trace_id,"register event listener, on={:?}",self.event);

                    raw.add_listener(self.event.clone(), cx.waker().clone());

                    Poll::Pending
                }
                poll => poll,
            }
        };

        // log::trace!(target: trace_id, "poll {:?}, unlock", self.event);

        let mut wakers = self.wakers.lock_mut();

        if let Some(waker) = wakers.pop_front() {
            // log::trace!("poll {:?}, unlock, wakeup other", self.event);
            waker.wake();
        }

        poll
    }
}

/// Register event handle with async fn
#[macro_export]
macro_rules! on {
    ($mediator: expr, $event: expr, $fut: expr) => {
        $mediator.on_poll(Event::A, |mediator_cx, cx| {
            use $crate::FutureExt;
            Box::pin($fut(mediator_cx)).poll_unpin(cx)
        })
    };
}

pub type LocalMediator<T, E> =
    Mediator<shared::LocalShared<SharedData<T, E>>, shared::LocalShared<VecDeque<Waker>>>;

pub type MutexMediator<T, E> =
    Mediator<shared::MutexShared<SharedData<T, E>>, shared::MutexShared<VecDeque<Waker>>>;

#[cfg(test)]
mod tests {
    use std::task::Poll;

    use futures::{
        executor::{block_on, LocalPool, ThreadPool},
        task::{LocalSpawnExt, SpawnExt},
        FutureExt,
    };

    use crate::{LocalMediator, MutexMediator, SharedData};

    #[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
    enum Event {
        A,
        B,
    }

    #[test]
    fn test_mediator() {
        let mediator: MutexMediator<i32, Event> = MutexMediator::new(1);

        let thread_pool = ThreadPool::builder().pool_size(10).create().unwrap();

        let mediator_cloned = mediator.clone();

        thread_pool.spawn_ok(async move {
            mediator_cloned
                .on_poll(Event::B, |mediator_cx, _| {
                    if *mediator_cx.value() == 1 {
                        *mediator_cx.value_mut() = 2;
                        mediator_cx.notify(Event::A);

                        return Poll::Ready(());
                    }

                    mediator_cx.notify(Event::A);

                    return Poll::Pending;
                })
                .await
        });

        let mediator_cloned = mediator.clone();

        let handle = thread_pool
            .spawn_with_handle(async move {
                mediator_cloned
                    .on_poll(Event::A, |mediator_cx, _| {
                        if *mediator_cx.value() == 1 {
                            mediator_cx.notify(Event::B);
                            return Poll::Pending;
                        }

                        return Poll::Ready(());
                    })
                    .await;
            })
            .unwrap();

        block_on(handle);

        assert_eq!(mediator.with(|data| data.value), 2);
    }

    #[test]
    fn test_mediator_async() {
        let mediator: LocalMediator<i32, Event> = LocalMediator::new(1);

        async fn assign_2(cx: &mut SharedData<i32, Event>) {
            *cx.value_mut() = 2;
        }

        let mediator_cloned = mediator.clone();

        block_on(async move { on!(mediator_cloned, Event::A, assign_2).await });

        assert_eq!(mediator.with(|value| value.value), 2);
    }

    #[test]
    fn test_async_drop() {
        // _ = pretty_env_logger::try_init_timed();
        struct MockAsyncDrop {
            fd: i32,
            mediator: LocalMediator<i32, i32>,
        }

        impl Drop for MockAsyncDrop {
            fn drop(&mut self) {
                self.mediator.with_mut(|shared| {
                    *shared.value_mut() = 2;
                    shared.notify(self.fd);
                })
            }
        }

        let mediator = LocalMediator::new(0);

        let mock = MockAsyncDrop {
            fd: 2,
            mediator: mediator.clone(),
        };

        let mut pool = LocalPool::new();

        pool.spawner()
            .spawn_local(async move {
                let _mock = mock;
                // drop mock instance.
            })
            .unwrap();

        async fn drop_fd(fd: i32) {
            log::trace!("drop fd: {}", fd);
        }

        pool.run_until(mediator.on_poll(2, |shared, cx| {
            if *shared.value() == 2 {
                return Box::pin(drop_fd(2)).poll_unpin(cx);
            }

            return Poll::Pending;
        }));
    }
}
