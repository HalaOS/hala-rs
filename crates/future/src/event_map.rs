use std::{
    borrow::Borrow,
    fmt::Debug,
    hash::Hash,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
    task::{Poll, Waker},
};

use dashmap::DashMap;
use hala_sync::{AsyncGuardMut, AsyncLockable};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EventMapError {
    #[error("Waiting operation canceled by user")]
    Cancel,
    #[error("Waiting operation canceled by EventMap to drop `EventMap` self")]
    Destroy,
}

/// waiter wakeup reason.
#[derive(Debug, Clone, Copy)]
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

#[derive(Debug)]
struct WakerWithReason {
    waker: Waker,
    reason: Arc<AtomicU8>,
}

impl WakerWithReason {
    fn wake(self, reason: Reason) {
        self.reason.store(reason.into(), Ordering::Release);
        self.waker.wake();
    }

    fn wake_by_ref(&self, reason: Reason) {
        self.reason.store(reason.into(), Ordering::Release);
        self.waker.wake_by_ref();
    }
}

/// The mediator of event notify for futures-aware enviroment.
#[derive(Debug)]
pub struct EventMap<E>
where
    E: Send + Eq + Hash,
{
    wakers: DashMap<E, WakerWithReason>,
}

impl<E> Drop for EventMap<E>
where
    E: Send + Eq + Hash,
{
    fn drop(&mut self) {
        for entry in self.wakers.iter() {
            entry.value().wake_by_ref(Reason::Destroy);
        }
    }
}

impl<E> Default for EventMap<E>
where
    E: Send + Eq + Hash,
{
    fn default() -> Self {
        Self {
            wakers: DashMap::new(),
        }
    }
}

impl<E> EventMap<E>
where
    E: Send + Eq + Hash + Debug + Clone,
{
    /// Only remove event waker, without wakeup it.
    pub fn wait_cancel<Q>(&self, event: Q)
    where
        Q: Borrow<E>,
    {
        self.wakers.remove(event.borrow());
    }

    /// Notify one event `E` on.
    pub fn notify_one<Q>(&self, event: Q, reason: Reason) -> bool
    where
        Q: Borrow<E>,
    {
        if let Some((_, waker)) = self.wakers.remove(event.borrow()) {
            log::trace!("{:?} wakeup", event.borrow());
            waker.wake(reason);
            true
        } else {
            log::trace!("{:?} wakeup -- not found", event.borrow());
            false
        }
    }

    /// Notify all event on in the providing `events` list
    pub fn notify_all<L: AsRef<[E]>>(&self, events: L, reason: Reason) {
        for event in events.as_ref() {
            self.notify_one(event, reason);
        }
    }

    /// Notify all event on in the providing `events` list
    pub fn notify_any(&self, reason: Reason) {
        let events = self
            .wakers
            .iter()
            .map(|pair| pair.key().clone())
            .collect::<Vec<_>>();

        self.notify_all(&events, reason);
    }

    pub fn wait<'a, Q, G>(&'a self, event: Q, guard: G) -> Wait<'a, E, G>
    where
        G: AsyncGuardMut<'a> + 'a,
        Q: Borrow<E>,
    {
        Wait {
            event: event.borrow().clone(),
            guard: Some(guard),
            event_map: self,
            reason: Arc::new(AtomicU8::new(Reason::None.into())),
        }
    }
}

pub struct Wait<'a, E, G>
where
    E: Send + Eq + Hash,
    G: AsyncGuardMut<'a> + 'a,
{
    event: E,
    guard: Option<G>,
    event_map: &'a EventMap<E>,
    reason: Arc<AtomicU8>,
}

impl<'a, E, G> std::future::Future for Wait<'a, E, G>
where
    E: Send + Eq + Hash + Clone + Unpin + Debug,
    G: AsyncGuardMut<'a> + Unpin + 'a,
{
    type Output = Result<(), EventMapError>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        log::trace!("Event map poll.");

        if let Some(guard) = self.guard.take() {
            // insert waker into waiting map.
            self.event_map.wakers.insert(
                self.event.clone(),
                WakerWithReason {
                    waker: cx.waker().clone(),
                    reason: self.reason.clone(),
                },
            );

            G::Locker::unlock(guard);

            log::trace!("event_map release guard, event={:?}", self.event);
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
        } else {
            return Poll::Ready(Ok(()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use futures::{executor::ThreadPool, task::SpawnExt};
    use hala_sync::AsyncSpinMutex;

    #[futures_test::test]
    async fn test_across_suspend_point() {
        let local_pool = ThreadPool::builder().pool_size(10).create().unwrap();

        let mediator = Arc::new(EventMap::<i32>::default());

        let shared = Arc::new(AsyncSpinMutex::new(1));

        let mediator_cloned = mediator.clone();

        let handle = local_pool
            .spawn_with_handle(async move {
                let shared = shared.lock().await;

                mediator_cloned.wait(1, shared).await.unwrap();
            })
            .unwrap();

        while !mediator.notify_one(1, Reason::On) {}

        handle.await;
    }
}
