use std::{
    cell::RefCell,
    collections::VecDeque,
    future::Future,
    ops,
    rc::Rc,
    sync::{Arc, Mutex, MutexGuard},
    task::Waker,
};

use crate::Shared;

/// `Shared` extension trait for asynchronization program.
pub trait AsyncShared: Shared {
    /// Future type created by [`lock_wait`](AsyncShared::lock_wait)
    type RefFuture<'a>: Future<Output = Self::Ref<'a>>
    where
        Self: 'a;

    /// Future type created by [`lock_mut_wait`](AsyncShared::lock_wait)
    type RefMutFuture<'a>: Future<Output = Self::RefMut<'a>>
    where
        Self: 'a;

    /// Create new future and wait locking shared value and getting immutable reference.
    fn lock_wait(&self) -> Self::RefFuture<'_>;

    /// Create new future and wait locking shared value and getting mutable reference.
    fn lock_mut_wait(&self) -> Self::RefMutFuture<'_>;
}

/// Shared data support local thread mode and `AsyncShared` trait.
pub struct AsyncLocalShared<T> {
    value: Rc<RefCell<T>>,
    wakers: Rc<RefCell<VecDeque<Waker>>>,
}

impl<T> AsyncLocalShared<T> {
    pub fn new(value: T) -> Self {
        Self {
            value: Rc::new(RefCell::new(value)),
            wakers: Default::default(),
        }
    }
}

impl<T> From<T> for AsyncLocalShared<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T> Clone for AsyncLocalShared<T> {
    fn clone(&self) -> Self {
        Self {
            value: Rc::clone(&self.value),
            wakers: Rc::clone(&self.wakers),
        }
    }
}

pub struct AsyncLocalSharedRef<'a, T> {
    value_ref: std::cell::Ref<'a, T>,
    wakers: Rc<RefCell<VecDeque<Waker>>>,
}

impl<'a, T> ops::Deref for AsyncLocalSharedRef<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value_ref
    }
}

impl<'a, T> Drop for AsyncLocalSharedRef<'a, T> {
    fn drop(&mut self) {
        if let Some(waker) = self.wakers.borrow_mut().pop_front() {
            waker.wake();
        }
    }
}

pub struct AsyncLocalSharedRefMut<'a, T> {
    value_ref: std::cell::RefMut<'a, T>,
    wakers: Rc<RefCell<VecDeque<Waker>>>,
}

impl<'a, T> ops::Deref for AsyncLocalSharedRefMut<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value_ref
    }
}

impl<'a, T> ops::DerefMut for AsyncLocalSharedRefMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value_ref
    }
}

impl<'a, T> Drop for AsyncLocalSharedRefMut<'a, T> {
    fn drop(&mut self) {
        if let Some(waker) = self.wakers.borrow_mut().pop_front() {
            waker.wake();
        }
    }
}

pub struct AsyncLocalSharedRefFuture<'a, T> {
    value: &'a AsyncLocalShared<T>,
}

impl<'a, T> Future for AsyncLocalSharedRefFuture<'a, T> {
    type Output = <AsyncLocalShared<T> as Shared>::Ref<'a>;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::task::Poll::Ready(self.value.lock())
    }
}

pub struct AsyncLocalSharedRefMutFuture<'a, T> {
    value: &'a AsyncLocalShared<T>,
}

impl<'a, T> Future for AsyncLocalSharedRefMutFuture<'a, T> {
    type Output = <AsyncLocalShared<T> as Shared>::RefMut<'a>;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::task::Poll::Ready(self.value.lock_mut())
    }
}

impl<T> Shared for AsyncLocalShared<T> {
    type Value = T;

    type Ref<'a> = AsyncLocalSharedRef<'a,T>
    where
        Self: 'a;

    type RefMut<'a> = AsyncLocalSharedRefMut<'a,T>
    where
        Self: 'a;

    fn lock(&self) -> Self::Ref<'_> {
        AsyncLocalSharedRef {
            value_ref: self.value.lock(),
            wakers: self.wakers.clone(),
        }
    }

    fn lock_mut(&self) -> Self::RefMut<'_> {
        AsyncLocalSharedRefMut {
            value_ref: self.value.lock_mut(),
            wakers: self.wakers.clone(),
        }
    }

    fn try_lock_mut(&self) -> Option<Self::RefMut<'_>> {
        self.value
            .try_lock_mut()
            .map(|ref_mut| AsyncLocalSharedRefMut {
                value_ref: ref_mut,
                wakers: self.wakers.clone(),
            })
    }
}

impl<T> AsyncShared for AsyncLocalShared<T> {
    type RefFuture<'a> = AsyncLocalSharedRefFuture<'a,T>
    where
        Self: 'a;

    type RefMutFuture<'a>= AsyncLocalSharedRefMutFuture<'a,T>
    where
        Self: 'a;

    fn lock_wait(&self) -> Self::RefFuture<'_> {
        AsyncLocalSharedRefFuture { value: self }
    }

    fn lock_mut_wait(&self) -> Self::RefMutFuture<'_> {
        AsyncLocalSharedRefMutFuture { value: self }
    }
}

/// Shared data support local thread mode and `AsyncShared` trait.
pub struct AsyncMutexShared<T> {
    value: Arc<Mutex<T>>,
    wakers: Arc<Mutex<VecDeque<Waker>>>,
}

impl<T> AsyncMutexShared<T> {
    pub fn new(value: T) -> Self {
        Self {
            value: Arc::new(Mutex::new(value)),
            wakers: Default::default(),
        }
    }
}

impl<T> From<T> for AsyncMutexShared<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T> Clone for AsyncMutexShared<T> {
    fn clone(&self) -> Self {
        Self {
            value: Arc::clone(&self.value),
            wakers: Arc::clone(&self.wakers),
        }
    }
}

pub struct AsyncMutexSharedRef<'a, T> {
    value_ref: MutexGuard<'a, T>,
    wakers: Arc<Mutex<VecDeque<Waker>>>,
}

impl<'a, T> ops::Deref for AsyncMutexSharedRef<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value_ref
    }
}

impl<'a, T> Drop for AsyncMutexSharedRef<'a, T> {
    fn drop(&mut self) {
        if let Some(waker) = self.wakers.lock_mut().pop_front() {
            waker.wake();
        }
    }
}

pub struct AsyncMutexSharedRefMut<'a, T> {
    value_ref: MutexGuard<'a, T>,
    wakers: Arc<Mutex<VecDeque<Waker>>>,
}

impl<'a, T> ops::Deref for AsyncMutexSharedRefMut<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value_ref
    }
}

impl<'a, T> ops::DerefMut for AsyncMutexSharedRefMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value_ref
    }
}

impl<'a, T> Drop for AsyncMutexSharedRefMut<'a, T> {
    fn drop(&mut self) {
        if let Some(waker) = self.wakers.lock_mut().pop_front() {
            waker.wake();
        }
    }
}

pub struct AsyncMutexSharedRefFuture<'a, T> {
    value: &'a AsyncMutexShared<T>,
}

impl<'a, T> Future for AsyncMutexSharedRefFuture<'a, T> {
    type Output = <AsyncMutexShared<T> as Shared>::Ref<'a>;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::task::Poll::Ready(self.value.lock())
    }
}

pub struct AsyncMutexSharedRefMutFuture<'a, T> {
    value: &'a AsyncMutexShared<T>,
}

impl<'a, T> Future for AsyncMutexSharedRefMutFuture<'a, T> {
    type Output = <AsyncMutexShared<T> as Shared>::RefMut<'a>;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::task::Poll::Ready(self.value.lock_mut())
    }
}

impl<T> Shared for AsyncMutexShared<T> {
    type Value = T;

    type Ref<'a> = AsyncMutexSharedRef<'a,T>
    where
        Self: 'a;

    type RefMut<'a> = AsyncMutexSharedRefMut<'a,T>
    where
        Self: 'a;

    fn lock(&self) -> Self::Ref<'_> {
        AsyncMutexSharedRef {
            value_ref: self.value.lock().unwrap(),
            wakers: self.wakers.clone(),
        }
    }

    fn lock_mut(&self) -> Self::RefMut<'_> {
        AsyncMutexSharedRefMut {
            value_ref: self.value.lock_mut(),
            wakers: self.wakers.clone(),
        }
    }

    fn try_lock_mut(&self) -> Option<Self::RefMut<'_>> {
        self.value
            .try_lock_mut()
            .map(|ref_mut| AsyncMutexSharedRefMut {
                value_ref: ref_mut,
                wakers: self.wakers.clone(),
            })
    }
}

impl<T> AsyncShared for AsyncMutexShared<T> {
    type RefFuture<'a> = AsyncMutexSharedRefFuture<'a,T>
    where
        Self: 'a;

    type RefMutFuture<'a>= AsyncMutexSharedRefMutFuture<'a,T>
    where
        Self: 'a;

    fn lock_wait(&self) -> Self::RefFuture<'_> {
        AsyncMutexSharedRefFuture { value: self }
    }

    fn lock_mut_wait(&self) -> Self::RefMutFuture<'_> {
        AsyncMutexSharedRefMutFuture { value: self }
    }
}

#[cfg(test)]
mod tests {

    use futures::{
        channel::oneshot::channel,
        executor::{LocalPool, ThreadPool},
        task::{LocalSpawnExt, SpawnExt},
    };

    use super::*;

    #[test]
    fn test_async_shared() {
        let mut local_pool = LocalPool::new();

        let shared = AsyncLocalShared::new(1);

        let shared_cloned = shared.clone();

        let (sx, rx) = channel::<()>();

        local_pool
            .spawner()
            .spawn_local(async move {
                let mut locked = shared_cloned.lock_mut_wait().await;

                *locked = 2;

                sx.send(()).unwrap();
            })
            .unwrap();

        local_pool.run_until(async move {
            rx.await.unwrap();

            let locked = shared.lock_mut_wait().await;

            assert_eq!(*locked, 2);
        });
    }

    #[futures_test::test]
    async fn test_async_shared_mutex() {
        let local_pool = ThreadPool::builder().pool_size(10).create().unwrap();

        let shared = AsyncMutexShared::new(1);

        let shared_cloned = shared.clone();

        let (sx, rx) = channel::<()>();

        local_pool
            .spawn(async move {
                let mut locked = shared_cloned.lock_mut_wait().await;

                *locked = 2;

                sx.send(()).unwrap();
            })
            .unwrap();

        rx.await.unwrap();

        let locked = shared.lock_mut_wait().await;

        assert_eq!(*locked, 2);
    }
}
