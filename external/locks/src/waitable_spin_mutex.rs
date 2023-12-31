use std::{
    collections::VecDeque,
    ops,
    task::{Context, Waker},
};

use super::*;

/// The type extend std [`Mutex`](std::sync::Mutex) type to support the `Locker`,`WaitableLocker` traits.
pub struct WaitableSpinMutex<T> {
    wakers: SpinMutex<VecDeque<Waker>>,
    spin_mutex: SpinMutex<T>,
}

impl<T> WaitableSpinMutex<T> {
    /// Creates a new mutex in an unlocked state ready for use.
    pub fn new(t: T) -> Self {
        Self {
            wakers: Default::default(),
            spin_mutex: SpinMutex::new(t),
        }
    }
}

impl<T> Locker for WaitableSpinMutex<T> {
    type Data = T;

    type Guard<'a> = WaitableSpinMutexGuard<'a,T>
    where
        Self: 'a,
        Self::Data: 'a;

    #[inline(always)]
    fn sync_lock(&self) -> Self::Guard<'_> {
        let guard = self.spin_mutex.sync_lock();

        WaitableSpinMutexGuard {
            guard: Some(guard),
            mutex: self,
        }
    }

    #[inline(always)]
    fn try_sync_lock(&self) -> Option<Self::Guard<'_>> {
        self.spin_mutex
            .try_sync_lock()
            .map(|guard| WaitableSpinMutexGuard {
                guard: Some(guard),
                mutex: self,
            })
    }
}

impl<T> WaitableLocker for WaitableSpinMutex<T> {
    type WaitableGuard<'a>= WaitableSpinMutexGuard<'a,T>
    where
        Self: 'a,
        Self::Data: 'a;

    #[inline(always)]
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

pub struct WaitableSpinMutexGuard<'a, T: 'a> {
    guard: Option<SpinMutexGuard<'a, T>>,
    mutex: &'a WaitableSpinMutex<T>,
}

impl<'a, T: 'a> Drop for WaitableSpinMutexGuard<'a, T> {
    fn drop(&mut self) {
        self.unlock();
    }
}

impl<'a, T: 'a> ops::Deref for WaitableSpinMutexGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.guard.as_deref().unwrap()
    }
}

impl<'a, T: 'a> ops::DerefMut for WaitableSpinMutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard.as_deref_mut().unwrap()
    }
}

impl<'a, T: 'a> LockerGuard<'a, T> for WaitableSpinMutexGuard<'a, T> {
    #[inline(always)]
    fn unlock(&mut self) {
        if let Some(guard) = self.guard.take() {
            drop(guard);

            if let Some(waker) = self.mutex.wakers.sync_lock().pop_front() {
                waker.wake();
            }
        }

        log::trace!("{:?} release lock", std::thread::current().id(),);
    }
}

impl<'a, T> WaitableLockerGuard<'a, T> for WaitableSpinMutexGuard<'a, T> {
    type Locker = WaitableSpinMutex<T>;

    fn locker_ref(&self) -> &'a Self::Locker {
        self.mutex
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use futures::{executor::ThreadPool, task::SpawnExt};

    use crate::{Locker, WaitableLocker, WaitableSpinMutex};

    struct Counter(usize);

    #[futures_test::test]
    async fn test_waitable_spin_mutex_async() {
        // pretty_env_logger::init_timed();

        let pool = ThreadPool::builder().pool_size(10).create().unwrap();

        let mutex = Arc::new(WaitableSpinMutex::new(Counter(0)));

        let tasks = 1000;

        let mut join_handles = vec![];

        for _ in 0..tasks {
            let mutex = mutex.clone();

            join_handles.push(
                pool.spawn_with_handle(async move {
                    for _ in 0..tasks {
                        let mut data = mutex.async_lock().await;
                        data.0 += 1;
                    }
                })
                .unwrap(),
            );
        }

        assert_eq!(join_handles.len(), tasks);

        for handle in join_handles {
            handle.await;
        }

        assert_eq!(mutex.async_lock().await.0, tasks * tasks);
    }

    #[test]
    fn test_waitable_spin_mutex() {
        let threads = 10;
        let tasks = 100000;

        let mutex = Arc::new(WaitableSpinMutex::new(0));

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
