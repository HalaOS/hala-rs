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
pub struct MediatorTM<Raw, Wakers> {
    raw: Raw,
    wakers: Wakers,
}

impl<Raw, Wakers> Clone for MediatorTM<Raw, Wakers>
where
    Raw: Clone,
    Wakers: Clone,
{
    fn clone(&self) -> Self {
        Self {
            raw: self.raw.clone(),
            wakers: self.wakers.clone(),
        }
    }
}

impl<T, E, Raw, Wakers> MediatorTM<Raw, Wakers>
where
    Raw: shared::Shared<Value = Shared<T, E>> + From<Shared<T, E>> + Clone,
    Wakers: shared::Shared<Value = VecDeque<Waker>> + From<VecDeque<Waker>> + Clone,
{
    /// Create new mediator with shared value.
    pub fn new(value: T) -> Self {
        Self {
            raw: Shared::new(value).into(),
            wakers: VecDeque::default().into(),
        }
    }

    /// Acquire the lock and access immutable shared data.
    pub fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        let raw = self.raw.lock();

        f(&raw.value)
    }

    /// Acquire the lock and access mutable shared data.
    pub fn with_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        let mut raw = self.raw.lock_mut();

        f(&mut raw.value)
    }

    /// Attempt to acquire the shared data lock immediately.
    ///
    /// If the lock is currently held, this will return `None`.
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
    pub async fn notify(&self, event: E)
    where
        E: Eq + Hash + Debug,
    {
        let mut raw = self.raw.lock_mut();

        raw.notify(event);
    }

    /// Emit all events
    pub async fn notify_all<Events: AsRef<[E]>>(&self, events: Events)
    where
        E: Eq + Hash + Clone + Debug,
    {
        let mut raw = self.raw.lock_mut();

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
    #[inline]
    pub fn on_fn<F, R>(&self, event: E, f: F) -> OnEvent<Raw, Wakers, E, F>
    where
        F: FnMut(&mut Shared<T, E>, &mut Context<'_>) -> Poll<R> + Unpin,
        T: Unpin + 'static,
        E: Unpin + Eq + Hash + Debug,
        R: Unpin,
    {
        log::trace!("call on_fn {:?}", event);

        OnEvent {
            f: Some(f),
            raw: self.raw.clone(),
            wakers: self.wakers.clone(),
            event,
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
}

impl<Raw, Wakers, E, F> Drop for OnEvent<Raw, Wakers, E, F>
where
    E: Debug,
{
    fn drop(&mut self) {
        log::trace!("Dropping OnEvent: {:?}", self.event);
    }
}

impl<Raw, Wakers, T, E, F, R> Future for OnEvent<Raw, Wakers, E, F>
where
    Raw: shared::Shared<Value = Shared<T, E>> + Unpin + Clone,
    Wakers: shared::Shared<Value = VecDeque<Waker>> + Unpin + Clone,
    F: FnMut(&mut Shared<T, E>, &mut Context<'_>) -> Poll<R> + Unpin,
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
        log::trace!("{:?} poll", self.event);

        let raw = self.raw.clone();

        let mut raw = match raw.try_lock_mut() {
            Some(raw) => raw,
            _ => {
                self.wakers.lock_mut().push_back(cx.waker().clone());
                return Poll::Pending;
            }
        };

        log::trace!("{:?} poll locked", self.event);

        let mut f = self.f.take().unwrap();

        match f(&mut raw, cx) {
            Poll::Pending => {
                self.f = Some(f);

                raw.register_event_listener(self.event.clone(), cx.waker().clone());

                log::trace!("{:?} poll unlock, returns pending", self.event);

                if let Some(waker) = self.wakers.lock_mut().pop_front() {
                    waker.wake();
                }

                return Poll::Pending;
            }
            poll => {
                log::trace!("{:?} poll unlock, returns ready", self.event);

                if let Some(waker) = self.wakers.lock_mut().pop_front() {
                    waker.wake();
                }

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

pub type LocalMediator<T, E> =
    MediatorTM<shared::LocalShared<Shared<T, E>>, shared::LocalShared<VecDeque<Waker>>>;
pub type MutexMediator<T, E> =
    MediatorTM<shared::MutexShared<Shared<T, E>>, shared::MutexShared<VecDeque<Waker>>>;

#[cfg(not(feature = "single-thread"))]
pub type Mediator<T, E> = MutexMediator<T, E>;

#[cfg(feature = "single-thread")]
pub type Mediator<T, E> = LocalMediator<T, E>;

#[cfg(test)]
mod tests {
    use std::task::Poll;

    use futures::executor::{block_on, ThreadPool};

    use futures::task::SpawnExt;

    use crate::{LocalMediator, MutexMediator, Shared};

    #[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
    enum Event {
        A,
        B,
    }

    #[futures_test::test]
    async fn test_mediator() {
        let mediator: MutexMediator<i32, Event> = MutexMediator::new(1);

        let thread_pool = ThreadPool::builder().pool_size(10).create().unwrap();

        let mediator_cloned = mediator.clone();

        thread_pool
            .spawn(async move {
                mediator_cloned
                    .on_fn(Event::B, |mediator_cx, _| {
                        if *mediator_cx.value() == 1 {
                            *mediator_cx.value_mut() = 2;
                            mediator_cx.notify(Event::A);

                            return Poll::Ready(());
                        }

                        return Poll::Pending;
                    })
                    .await
            })
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

    #[test]
    fn test_mediator_async() {
        let mediator: LocalMediator<i32, Event> = LocalMediator::new(1);

        async fn assign_2(cx: &mut Shared<i32, Event>) {
            *cx.value_mut() = 2;
        }

        let mediator_cloned = mediator.clone();

        block_on(async move { on!(mediator_cloned, Event::A, assign_2).await });

        assert_eq!(mediator.with(|value| *value), 2);
    }
}
