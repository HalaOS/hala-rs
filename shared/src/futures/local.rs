use std::{cell::RefCell, collections::VecDeque, ops, rc::Rc, task::Waker};

use std::future::Future;

use crate::{AsyncSharedGuard, AsyncSharedGuardMut, SharedGuard, SharedGuardMut};

use super::{AsyncShared, Shared};

/// Shared data support local thread mode and `AsyncShared` trait.
pub struct AsyncLocalShared<T> {
    value: Rc<RefCell<T>>,
    pub(super) wakers: Rc<RefCell<VecDeque<Waker>>>,
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
    type Output = AsyncSharedGuard<'a, AsyncLocalShared<T>>;
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
            self.value.wakers.borrow_mut().push_back(cx.waker().clone());
            std::task::Poll::Pending
        }
    }
}

pub struct AsyncLocalSharedRefMutFuture<'a, T> {
    value: &'a AsyncLocalShared<T>,
}

impl<'a, T> Future for AsyncLocalSharedRefMutFuture<'a, T> {
    type Output = AsyncSharedGuardMut<'a, AsyncLocalShared<T>>;
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
            self.value.wakers.borrow_mut().push_back(cx.waker().clone());
            std::task::Poll::Pending
        }
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

    fn lock(&self) -> SharedGuard<'_, Self> {
        SharedGuard {
            value: Some(AsyncLocalSharedRef {
                value_ref: self.value.borrow(),
                wakers: self.wakers.clone(),
            }),
            shared: self,
        }
    }

    fn lock_mut(&self) -> SharedGuardMut<'_, Self> {
        SharedGuardMut {
            value: Some(AsyncLocalSharedRefMut {
                value_ref: self.value.borrow_mut(),
                wakers: self.wakers.clone(),
            }),
            shared: self,
        }
    }

    fn try_lock_mut(&self) -> Option<SharedGuardMut<'_, Self>> {
        match self.value.try_borrow_mut() {
            Ok(value) => Some(SharedGuardMut {
                value: Some(AsyncLocalSharedRefMut {
                    value_ref: value,
                    wakers: self.wakers.clone(),
                }),
                shared: self,
            }),
            Err(_) => None,
        }
    }

    fn try_lock(&self) -> Option<SharedGuard<'_, Self>> {
        match self.value.try_borrow() {
            Ok(value) => Some(SharedGuard {
                value: Some(AsyncLocalSharedRef {
                    value_ref: value,
                    wakers: self.wakers.clone(),
                }),
                shared: self,
            }),
            Err(_) => None,
        }
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

#[cfg(test)]
mod tests {
    use futures::FutureExt;
    use futures_test::task::noop_context;
    use std::task::Poll;

    use crate::{AsyncLocalShared, AsyncShared};

    #[test]
    fn test_lock_wait() {
        let shared = AsyncLocalShared::new(1);

        let polling = Box::pin(shared.lock_wait()).poll_unpin(&mut noop_context());

        assert!(polling.is_ready());

        assert_eq!(shared.wakers.borrow().len(), 0);

        if let Poll::Ready(_shared_unlocked) = polling {
            let polling = Box::pin(shared.lock_wait()).poll_unpin(&mut noop_context());

            assert!(polling.is_ready());

            assert_eq!(shared.wakers.borrow().len(), 0);
        }
    }

    #[test]
    fn test_lock_mut_wait() {
        let shared = AsyncLocalShared::new(1);

        let polling = Box::pin(shared.lock_mut_wait()).poll_unpin(&mut noop_context());

        assert!(polling.is_ready());

        assert_eq!(shared.wakers.borrow().len(), 0);

        if let Poll::Ready(_shared_unlocked) = polling {
            let polling = Box::pin(shared.lock_mut_wait()).poll_unpin(&mut noop_context());

            assert!(polling.is_pending());

            assert_eq!(shared.wakers.borrow().len(), 1);
        }

        assert_eq!(shared.wakers.borrow().len(), 0);

        let polling = Box::pin(shared.lock_mut_wait()).poll_unpin(&mut noop_context());

        assert!(polling.is_ready());

        assert_eq!(shared.wakers.borrow().len(), 0);
    }
}
