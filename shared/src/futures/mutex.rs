use crate::{
    AsyncShared, AsyncSharedGuard, AsyncSharedGuardMut, Shared, SharedGuard, SharedGuardMut,
};
use std::{
    collections::VecDeque,
    future::Future,
    ops,
    sync::{Arc, Mutex, MutexGuard},
    task::Waker,
};

/// Shared data support local thread mode and `AsyncShared` trait.
pub struct AsyncMutexShared<T> {
    value: Arc<Mutex<T>>,
    pub(super) wakers: Arc<Mutex<VecDeque<Waker>>>,
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
    value_ref: Option<MutexGuard<'a, T>>,
    wakers: Arc<Mutex<VecDeque<Waker>>>,
}

impl<'a, T> ops::Deref for AsyncMutexSharedRef<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value_ref.as_deref().unwrap()
    }
}

impl<'a, T> Drop for AsyncMutexSharedRef<'a, T> {
    fn drop(&mut self) {
        drop(self.value_ref.take());
        if let Some(waker) = self.wakers.lock().unwrap().pop_front() {
            waker.wake();
        }
    }
}

pub struct AsyncMutexSharedRefMut<'a, T> {
    value_ref: Option<MutexGuard<'a, T>>,
    wakers: Arc<Mutex<VecDeque<Waker>>>,
}

impl<'a, T> ops::Deref for AsyncMutexSharedRefMut<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value_ref.as_deref().unwrap()
    }
}

impl<'a, T> ops::DerefMut for AsyncMutexSharedRefMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value_ref.as_deref_mut().unwrap()
    }
}

impl<'a, T> Drop for AsyncMutexSharedRefMut<'a, T> {
    fn drop(&mut self) {
        drop(self.value_ref.take());

        if let Some(waker) = self.wakers.lock().unwrap().pop_front() {
            waker.wake();
        }
    }
}

pub struct AsyncMutexSharedRefFuture<'a, T> {
    value: &'a AsyncMutexShared<T>,
}

impl<'a, T> Future for AsyncMutexSharedRefFuture<'a, T> {
    type Output = AsyncSharedGuard<'a, AsyncMutexShared<T>>;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        if let Some(lock) = self.value.try_lock() {
            std::task::Poll::Ready(AsyncSharedGuard {
                value: lock.value,
                shared: lock.shared,
            })
        } else {
            let mut wakers = self.value.wakers.lock().unwrap();

            // double check mut status
            if let Some(lock) = self.value.try_lock() {
                return std::task::Poll::Ready(AsyncSharedGuard {
                    value: lock.value,
                    shared: lock.shared,
                });
            }

            wakers.push_back(cx.waker().clone());

            std::task::Poll::Pending
        }
    }
}

pub struct AsyncMutexSharedRefMutFuture<'a, T> {
    value: &'a AsyncMutexShared<T>,
}

impl<'a, T> Future for AsyncMutexSharedRefMutFuture<'a, T> {
    type Output = AsyncSharedGuardMut<'a, AsyncMutexShared<T>>;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        if let Some(lock) = self.value.try_lock_mut() {
            std::task::Poll::Ready(AsyncSharedGuardMut {
                value: lock.value,
                shared: lock.shared,
            })
        } else {
            let mut wakers = self.value.wakers.lock().unwrap();

            // double check mut status
            if let Some(lock) = self.value.try_lock_mut() {
                return std::task::Poll::Ready(AsyncSharedGuardMut {
                    value: lock.value,
                    shared: lock.shared,
                });
            }

            wakers.push_back(cx.waker().clone());

            std::task::Poll::Pending
        }
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
    fn lock(&self) -> SharedGuard<'_, Self> {
        SharedGuard {
            value: Some(AsyncMutexSharedRef {
                value_ref: Some(self.value.lock().unwrap()),
                wakers: self.wakers.clone(),
            }),
            shared: self,
        }
    }

    fn lock_mut(&self) -> SharedGuardMut<'_, Self> {
        SharedGuardMut {
            value: Some(AsyncMutexSharedRefMut {
                value_ref: Some(self.value.lock().unwrap()),
                wakers: self.wakers.clone(),
            }),
            shared: self,
        }
    }

    fn try_lock_mut(&self) -> Option<SharedGuardMut<'_, Self>> {
        match self.value.try_lock() {
            Ok(value) => Some(SharedGuardMut {
                value: Some(AsyncMutexSharedRefMut {
                    value_ref: Some(value),
                    wakers: self.wakers.clone(),
                }),
                shared: self,
            }),
            Err(_) => None,
        }
    }

    fn try_lock(&self) -> Option<SharedGuard<'_, Self>> {
        match self.value.try_lock() {
            Ok(value) => Some(SharedGuard {
                value: Some(AsyncMutexSharedRef {
                    value_ref: Some(value),
                    wakers: self.wakers.clone(),
                }),
                shared: self,
            }),
            Err(_) => None,
        }
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
    use futures::FutureExt;
    use futures_test::task::noop_context;
    use std::task::Poll;

    use crate::{AsyncMutexShared, AsyncShared};

    #[test]
    fn test_lock_wait() {
        let shared = AsyncMutexShared::new(1);

        let polling = Box::pin(shared.lock_wait()).poll_unpin(&mut noop_context());

        assert!(polling.is_ready());

        assert_eq!(shared.wakers.lock().unwrap().len(), 0);

        if let Poll::Ready(_shared_unlocked) = polling {
            let polling = Box::pin(shared.lock_wait()).poll_unpin(&mut noop_context());

            assert!(polling.is_pending());

            assert_eq!(shared.wakers.lock().unwrap().len(), 1);
        }

        assert_eq!(shared.wakers.lock().unwrap().len(), 0);

        let polling = Box::pin(shared.lock_wait()).poll_unpin(&mut noop_context());

        assert!(polling.is_ready());

        assert_eq!(shared.wakers.lock().unwrap().len(), 0);
    }

    #[test]
    fn test_lock_mut_wait() {
        let shared = AsyncMutexShared::new(1);

        let polling = Box::pin(shared.lock_mut_wait()).poll_unpin(&mut noop_context());

        assert!(polling.is_ready());

        assert_eq!(shared.wakers.lock().unwrap().len(), 0);

        if let Poll::Ready(_shared_unlocked) = polling {
            let polling = Box::pin(shared.lock_mut_wait()).poll_unpin(&mut noop_context());

            assert!(polling.is_pending());

            assert_eq!(shared.wakers.lock().unwrap().len(), 1);
        }

        assert_eq!(shared.wakers.lock().unwrap().len(), 0);

        let polling = Box::pin(shared.lock_mut_wait()).poll_unpin(&mut noop_context());

        assert!(polling.is_ready());

        assert_eq!(shared.wakers.lock().unwrap().len(), 0);
    }
}
