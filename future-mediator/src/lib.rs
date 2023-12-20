use std::{
    collections::VecDeque,
    fmt::Debug,
    future::Future,
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex},
    task::Context,
};

use std::{
    collections::HashMap,
    hash::Hash,
    task::{Poll, Waker},
};

pub use futures::FutureExt;

use std::{cell::RefCell, rc::Rc};

pub trait ThreadModel {
    type Guard<T>: ThreadModelGuard<T> + Unpin + Clone + From<T>;
}

pub trait ThreadModelGuard<T> {
    type Ref<'a, V>: Deref<Target = V>
    where
        Self: 'a,
        V: 'a;

    type RefMut<'a, V>: DerefMut<Target = V>
    where
        Self: 'a,
        V: 'a;

    fn new(value: T) -> Self;
    /// Get immutable reference of type `T`
    fn get(&self) -> Self::Ref<'_, T>;

    /// Get mutable reference of type `T`
    fn get_mut(&self) -> Self::RefMut<'_, T>;

    fn try_get_mut(&self) -> Option<Self::RefMut<'_, T>>;

    fn is_multithread() -> bool;
}

/// Single thread model
pub struct STModel;

pub struct STModelHolder<T> {
    /// protected value.
    value: Rc<RefCell<T>>,
}

impl<T> Clone for STModelHolder<T> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
        }
    }
}

unsafe impl<T> Sync for STModelHolder<T> {}
unsafe impl<T> Send for STModelHolder<T> {}

impl<T> From<T> for STModelHolder<T> {
    fn from(value: T) -> Self {
        STModelHolder::new(value)
    }
}

impl<T> ThreadModelGuard<T> for STModelHolder<T> {
    type Ref<'a,V> = std::cell::Ref<'a,V> where Self:'a,V: 'a;

    type RefMut<'a,V> = std::cell::RefMut<'a,V> where Self:'a,V: 'a;

    fn new(value: T) -> Self {
        Self {
            value: Rc::new(RefCell::new(value)),
        }
    }

    fn get(&self) -> std::cell::Ref<'_, T> {
        self.value.borrow()
    }

    fn get_mut(&self) -> std::cell::RefMut<'_, T> {
        self.value.borrow_mut()
    }

    fn try_get_mut(&self) -> Option<Self::RefMut<'_, T>> {
        Some(self.get_mut())
    }

    fn is_multithread() -> bool {
        false
    }
}

impl ThreadModel for STModel {
    type Guard<T> = STModelHolder<T>;
}

/// Multi thread model
pub struct MTModel;

pub struct MTModelHolder<T> {
    /// protected value.
    value: Arc<Mutex<T>>,
}

impl<T> Clone for MTModelHolder<T> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
        }
    }
}

impl<T> From<T> for MTModelHolder<T> {
    fn from(value: T) -> Self {
        MTModelHolder::new(value)
    }
}

impl<T> ThreadModelGuard<T> for MTModelHolder<T> {
    type Ref<'a,V> = std::sync::MutexGuard<'a,V> where Self:'a,V: 'a;

    type RefMut<'a,V> = std::sync::MutexGuard<'a,V> where Self:'a,V: 'a;

    fn new(value: T) -> Self {
        Self {
            value: Arc::new(Mutex::new(value)),
        }
    }

    fn get(&self) -> std::sync::MutexGuard<'_, T> {
        self.value.lock().unwrap()
    }

    fn get_mut(&self) -> std::sync::MutexGuard<'_, T> {
        self.value.lock().unwrap()
    }

    fn try_get_mut(&self) -> Option<Self::RefMut<'_, T>> {
        match self.value.try_lock() {
            Ok(value) => Some(value),
            _ => None,
        }
    }

    fn is_multithread() -> bool {
        true
    }
}

impl ThreadModel for MTModel {
    type Guard<T> = MTModelHolder<T>;
}

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
pub struct MediatorTM<TM: ThreadModel, T, E> {
    raw: TM::Guard<Shared<T, E>>,
    wakers: TM::Guard<VecDeque<Waker>>,
}

impl<TM: ThreadModel, T, E> Clone for MediatorTM<TM, T, E> {
    fn clone(&self) -> Self {
        Self {
            raw: self.raw.clone(),
            wakers: self.wakers.clone(),
        }
    }
}

impl<TM: ThreadModel, T, E> MediatorTM<TM, T, E> {
    /// Create new mediator with shared value.
    pub fn new(value: T) -> Self {
        Self {
            raw: Shared::new(value).into(),
            wakers: VecDeque::default().into(),
        }
    }

    /// Acquire the lock and access immutable shared data.
    pub async fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        let raw = self.raw.get();

        f(&raw.value)
    }

    /// Acquire the lock and access mutable shared data.
    pub async fn with_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        let mut raw = self.raw.get_mut();

        f(&mut raw.value)
    }

    /// Attempt to acquire the shared data lock immediately.
    ///
    /// If the lock is currently held, this will return `None`.
    pub fn try_lock_with<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&mut T) -> R,
    {
        if let Some(mut locked) = self.raw.try_get_mut() {
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
        let mut raw = self.raw.get_mut();

        raw.notify(event);
    }

    /// Emit all events
    pub async fn notify_all<Events: AsRef<[E]>>(&self, events: Events)
    where
        E: Eq + Hash + Clone + Debug,
    {
        let mut raw = self.raw.get_mut();

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
    pub fn on_fn<F, R>(&self, event: E, f: F) -> OnEvent<TM, T, E, F>
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
pub struct OnEvent<TM: ThreadModel, T, E, F>
where
    E: Debug,
{
    f: Option<F>,
    raw: TM::Guard<Shared<T, E>>,
    wakers: TM::Guard<VecDeque<Waker>>,
    event: E,
}

impl<TM: ThreadModel, T, E, F> Drop for OnEvent<TM, T, E, F>
where
    E: Debug,
{
    fn drop(&mut self) {
        log::trace!("Dropping OnEvent: {:?}", self.event);
    }
}

impl<TM: ThreadModel, T, E, F, R> Future for OnEvent<TM, T, E, F>
where
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

        let mut raw = match raw.try_get_mut() {
            Some(raw) => raw,
            _ => {
                self.wakers.get_mut().push_back(cx.waker().clone());
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

                if let Some(waker) = self.wakers.get_mut().pop_front() {
                    waker.wake();
                }

                return Poll::Pending;
            }
            poll => {
                log::trace!("{:?} poll unlock, returns ready", self.event);

                if let Some(waker) = self.wakers.get_mut().pop_front() {
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

pub type STMediator<T, E> = MediatorTM<STModel, T, E>;
pub type MTMediator<T, E> = MediatorTM<MTModel, T, E>;

#[cfg(not(feature = "single-thread"))]
pub type Mediator<T, E> = MTMediator<T, E>;

#[cfg(feature = "single-thread")]
pub type Mediator<T, E> = STMediator<T, E>;

#[cfg(test)]
mod tests {
    use std::task::Poll;

    use futures::executor::ThreadPool;

    use futures::task::SpawnExt;

    use crate::{Mediator, STMediator, Shared};

    #[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
    enum Event {
        A,
        B,
    }

    #[futures_test::test]
    async fn test_mediator() {
        let mediator: Mediator<i32, Event> = Mediator::new(1);

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

    #[futures_test::test]
    async fn test_mediator_async() {
        let mediator: STMediator<i32, Event> = STMediator::new(1);

        let thread_pool = ThreadPool::builder().pool_size(10).create().unwrap();

        async fn assign_2(cx: &mut Shared<i32, Event>) {
            *cx.value_mut() = 2;
        }

        let mediator_cloned = mediator.clone();

        thread_pool
            .spawn_with_handle(async move { on!(mediator_cloned, Event::A, assign_2).await })
            .unwrap()
            .await;

        assert_eq!(mediator.with(|value| *value).await, 2);
    }
}
