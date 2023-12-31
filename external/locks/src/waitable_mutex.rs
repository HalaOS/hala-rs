use std::{
    collections::VecDeque,
    ops,
    sync::TryLockError,
    task::{Context, Waker},
};

use super::*;

/// The type extend std [`Mutex`](std::sync::Mutex) type to support the `Locker`,`WaitableLocker` traits.
pub struct WaitableMutex<T: ?Sized> {
    wakers: SpinMutex<VecDeque<Waker>>,
    std_mutex: std::sync::Mutex<T>,
}

impl<T> WaitableMutex<T> {
    /// Creates a new mutex in an unlocked state ready for use.
    pub fn new(t: T) -> Self {
        Self {
            wakers: Default::default(),
            std_mutex: std::sync::Mutex::new(t),
        }
    }
}

impl<T> Locker for WaitableMutex<T> {
    type Data = T;

    type Guard<'a> = MutexWaitableLockerGuard<'a,T>
    where
        Self: 'a,
        Self::Data: 'a;

    fn sync_lock(&self) -> Self::Guard<'_> {
        let std_guard = self.std_mutex.lock().unwrap();

        MutexWaitableLockerGuard {
            std_guard: Some(std_guard),
            mutex: self,
        }
    }

    fn try_sync_lock(&self) -> Option<Self::Guard<'_>> {
        match self.std_mutex.try_lock() {
            Ok(guard) => Some(MutexWaitableLockerGuard {
                std_guard: Some(guard),
                mutex: self,
            }),
            Err(TryLockError::Poisoned(err)) => Some(MutexWaitableLockerGuard {
                std_guard: Some(err.into_inner()),
                mutex: self,
            }),
            Err(_) => None,
        }
    }
}

impl<T> WaitableLocker for WaitableMutex<T> {
    type WaitableGuard<'a>= MutexWaitableLockerGuard<'a,T>
    where
        Self: 'a,
        Self::Data: 'a;

    fn try_lock_with_context(&self, cx: &mut Context<'_>) -> Option<Self::WaitableGuard<'_>> {
        let mut wakers = self.wakers.sync_lock();

        match self.try_sync_lock() {
            Some(guard) => Some(guard),
            None => {
                wakers.push_back(cx.waker().clone());

                None
            }
        }
    }
}

pub struct MutexWaitableLockerGuard<'a, T: ?Sized + 'a> {
    std_guard: Option<std::sync::MutexGuard<'a, T>>,
    mutex: &'a WaitableMutex<T>,
}

impl<'a, T: ?Sized + 'a> Drop for MutexWaitableLockerGuard<'a, T> {
    fn drop(&mut self) {
        self.unlock();
    }
}

impl<'a, T: ?Sized + 'a> ops::Deref for MutexWaitableLockerGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.std_guard.as_deref().unwrap()
    }
}

impl<'a, T: ?Sized + 'a> ops::DerefMut for MutexWaitableLockerGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.std_guard.as_deref_mut().unwrap()
    }
}

impl<'a, T: ?Sized + 'a> LockerGuard<'a, T> for MutexWaitableLockerGuard<'a, T> {
    #[inline(always)]
    fn unlock(&mut self) {
        if let Some(guard) = self.std_guard.take() {
            drop(guard);

            if let Some(waker) = self.mutex.wakers.sync_lock().pop_front() {
                waker.wake();
            }
        }
    }

    #[inline(always)]
    fn sync_relock(&mut self) {
        let std_guard = match self.mutex.std_mutex.lock() {
            Ok(guard) => guard,
            Err(err) => err.into_inner(),
        };

        self.std_guard = Some(std_guard);
    }

    #[inline(always)]
    fn try_sync_relock(&mut self) -> bool {
        let std_guard = match self.mutex.std_mutex.try_lock() {
            Ok(guard) => guard,
            // Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(_) => return false,
        };

        self.std_guard = Some(std_guard);

        return true;
    }
}

impl<'a, T> WaitableLockerGuard<'a, T> for MutexWaitableLockerGuard<'a, T> {
    type Locker = WaitableMutex<T>;

    fn locker_ref(&self) -> &'a Self::Locker {
        self.mutex
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use futures::{executor::ThreadPool, task::SpawnExt};

    use crate::{Locker, WaitableLocker, WaitableMutex};

    #[futures_test::test]
    async fn test_waitable_mutex_async() {
        let pool = ThreadPool::builder().pool_size(10).create().unwrap();

        let mutex = Arc::new(WaitableMutex::new(0));

        let tasks = 1000;

        let mut join_handles = vec![];

        for _ in 0..tasks {
            let mutex = mutex.clone();
            join_handles.push(
                pool.spawn_with_handle(async move {
                    for _ in 0..tasks {
                        let mut data = mutex.async_lock().await;

                        *data = *data + 1;
                    }
                })
                .unwrap(),
            );
        }

        for handle in join_handles {
            handle.await;
        }

        assert_eq!(*mutex.async_lock().await, tasks * tasks);
    }

    #[test]
    fn test_waitable_mutex() {
        let threads = 10;
        let tasks = 100000;

        let mutex = Arc::new(WaitableMutex::new(0));

        let mut join_handles = vec![];

        for _ in 0..threads {
            let mutex = mutex.clone();
            join_handles.push(std::thread::spawn(move || {
                for _ in 0..tasks {
                    let mut data = mutex.sync_lock();

                    *data = *data + 1;
                }
            }));
        }

        for handle in join_handles {
            handle.join().unwrap();
        }

        assert_eq!(*mutex.sync_lock(), tasks * threads);
    }
}
