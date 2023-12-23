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

    fn register_event_listener(&mut self, event: E, waker: Waker)
    where
        E: Eq + Hash + Debug,
    {
        log::trace!("register event={:?}", event);

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
pub trait Mediator {
    type Value: Unpin + 'static;
    type Event: Eq + Hash + Clone + Unpin + Debug;

    type OnEvent<F, R>: Future<Output = R>
    where
        F: FnMut(&mut SharedData<Self::Value, Self::Event>, &mut Context<'_>) -> Poll<R> + Unpin,
        R: Unpin;

    /// Create new `Mediator` instance with shared value.
    fn new(value: Self::Value) -> Self;

    /// Acquire the lock and access immutable shared data.
    fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&SharedData<Self::Value, Self::Event>) -> R;

    /// Acquire the lock and access mutable shared data.
    fn with_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut SharedData<Self::Value, Self::Event>) -> R;

    /// Attempt to acquire the shared data lock immediately.
    ///
    /// If the lock is currently held, this will return `None`.
    fn try_lock_with<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&mut Self::Value) -> R;

    /// Notify one listener event on
    fn notify(&self, event: Self::Event);

    /// Notify listeners events on
    fn notify_all<Events: AsRef<[Self::Event]>>(&self, events: Events);

    /// Create a new event handle future with poll function `f` and run once immediately
    ///
    /// If `f` returns [`Pending`](Poll::Pending), the system will move the
    /// handle into the event waiting map, and the future returns `Pending` status.
    ///
    /// You can call [`notify`](Mediator::notify) to wake up this poll function and run once again.
    fn on_poll<F, R>(&self, event: Self::Event, f: F) -> Self::OnEvent<F, R>
    where
        F: FnMut(&mut SharedData<Self::Value, Self::Event>, &mut Context<'_>) -> Poll<R> + Unpin,
        R: Unpin;
}

/// A mediator is a central hub for communication between futures.
pub struct Hub<Raw, Wakers> {
    raw: Raw,
    wakers: Wakers,
    trace_id: Option<&'static str>,
}

impl<Raw, Wakers> Hub<Raw, Wakers> {}

impl<Raw, Wakers> Default for Hub<Raw, Wakers>
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

impl<Raw, Wakers> Clone for Hub<Raw, Wakers>
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

impl<T, E, Raw, Wakers> Hub<Raw, Wakers>
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

    pub fn new_with(value: T, trace_id: &'static str) -> Self {
        Self {
            raw: SharedData::new(value).into(),
            wakers: VecDeque::default().into(),
            trace_id: Some(trace_id),
        }
    }

    pub fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&SharedData<T, E>) -> R,
    {
        let raw = self.raw.lock();

        // log::trace!(target:self.trace_id.unwrap_or(""), "with locked");

        let r = f(&raw);

        // log::trace!(target:self.trace_id.unwrap_or(""), "with unlock");

        let mut wakers = self.wakers.lock_mut();

        if let Some(waker) = wakers.pop_front() {
            // log::trace!(target:self.trace_id.unwrap_or(""), "with unlock, wakeup another");
            waker.wake();
        }

        r
    }

    /// Acquire the lock and access mutable shared data.
    pub fn with_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut SharedData<T, E>) -> R,
    {
        let mut raw = self.raw.lock_mut();

        // log::trace!(target:self.trace_id.unwrap_or(""), "with_mut locked");

        let r = f(&mut raw);

        // log::trace!(target:self.trace_id.unwrap_or(""), "with_mut unlock");

        let mut wakers = self.wakers.lock_mut();

        if let Some(waker) = wakers.pop_front() {
            // log::trace!(target:self.trace_id.unwrap_or(""), "with_mut unlock, wakeup another");
            waker.wake();
        }

        r
    }

    pub fn try_lock_with<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&mut T) -> R,
    {
        if let Some(mut locked) = self.raw.try_lock_mut() {
            Some(f(&mut locked))
        } else {
            None
        }
    }

    /// Emit one event on.
    pub fn notify(&self, event: E)
    where
        E: Eq + Hash + Debug,
    {
        let mut raw = self.raw.lock_mut();

        raw.notify(event);
    }

    /// Emit all events
    pub fn notify_all<Events: AsRef<[E]>>(&self, events: Events) {
        let mut raw = self.raw.lock_mut();

        for event in events.as_ref() {
            raw.notify(event.clone());
        }
    }

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
        // let trace_id = self.trace_id.unwrap_or("");

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

                    raw.register_event_listener(self.event.clone(), cx.waker().clone());

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
    Hub<shared::LocalShared<SharedData<T, E>>, shared::LocalShared<VecDeque<Waker>>>;

pub type MutexMediator<T, E> =
    Hub<shared::MutexShared<SharedData<T, E>>, shared::MutexShared<VecDeque<Waker>>>;

#[cfg(test)]
mod tests {
    use std::task::Poll;

    use futures::{
        executor::{block_on, ThreadPool},
        task::SpawnExt,
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
}
