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

/// A mediator is a central hub for communication between futures.
#[derive(Clone)]
pub struct Mediator<T, E> {
    raw: Arc<Mutex<MediatorContext<T, E>>>,
}

pub struct MediatorContext<T, E> {
    value: T,
    wakers: HashMap<E, Waker>,
}

impl<T, E> MediatorContext<T, E> {
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

    pub fn value(&self) -> &T {
        &self.value
    }

    pub fn value_mut(&mut self) -> &mut T {
        &mut self.value
    }
}

impl<T, E> Deref for MediatorContext<T, E> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T, E> DerefMut for MediatorContext<T, E> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl<T, E> Mediator<T, E> {
    /// Create new mediator with shared value.
    pub fn new(value: T) -> Self {
        Self {
            raw: Arc::new(Mutex::new(MediatorContext::new(value))),
        }
    }

    /// notify one event on.
    pub async fn notify(&self, event: E)
    where
        E: Eq + Hash + Debug,
    {
        let mut raw = self.raw.lock().await;

        raw.notify(event);
    }

    /// Notify all events
    pub async fn notify_all<Events: AsRef<[E]>>(&self, events: Events)
    where
        E: Eq + Hash + Clone + Debug,
    {
        let mut raw = self.raw.lock().await;

        for event in events.as_ref() {
            raw.notify(event.clone());
        }
    }

    pub fn on<F, R>(&self, event: E, f: F) -> OnEvent<T, E, F>
    where
        F: FnMut(&mut MediatorContext<T, E>, &mut Context<'_>) -> Poll<R> + Unpin,
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

pub struct OnEvent<T, E, F>
where
    E: Debug,
{
    f: Option<F>,
    raw: Arc<Mutex<MediatorContext<T, E>>>,
    lock_future: Option<OwnedMutexLockFuture<MediatorContext<T, E>>>,
    event: E,
}

impl<T, E, F, R> Future for OnEvent<T, E, F>
where
    F: FnMut(&mut MediatorContext<T, E>, &mut Context<'_>) -> Poll<R> + Unpin,
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

#[cfg(test)]
mod tests {
    use std::task::Poll;

    use futures::executor::ThreadPool;

    use crate::Mediator;

    #[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
    enum Event {
        A,
        B,
    }

    #[futures_test::test]
    async fn test_mediator() {
        pretty_env_logger::init_timed();

        let mediator: Mediator<i32, Event> = Mediator::new(1);

        let thread_pool = ThreadPool::builder().pool_size(10).create().unwrap();

        thread_pool.spawn_ok(mediator.on(Event::B, |mediator_cx, _| {
            if *mediator_cx.value() == 1 {
                *mediator_cx.value_mut() = 2;
                mediator_cx.notify(Event::A);

                return Poll::Ready(());
            }

            return Poll::Pending;
        }));

        mediator
            .on(Event::A, |mediator_cx, _| {
                if *mediator_cx.value() == 1 {
                    return Poll::Pending;
                }

                return Poll::Ready(());
            })
            .await;
    }
}
