use std::{collections::VecDeque, ops, task::Waker};

use crate::{Locker, LockerGuard};

/// Spin mutex implementation with [`AtomicBool`]
#[derive(Debug)]
pub struct SpinMutex<T> {
    mutex: parking_lot::Mutex<T>,
}

impl<T: ?Sized + Default> Default for SpinMutex<T> {
    fn default() -> Self {
        Self {
            mutex: Default::default(),
        }
    }
}

// these are the only places where `T: Send` matters; all other
// functionality works fine on a single thread.
unsafe impl<T> Send for SpinMutex<T> {}

unsafe impl<T: Send> Sync for SpinMutex<T> {}

impl<T> Locker for SpinMutex<T> {
    type Data = T;

    type Guard<'a> = SpinMutexGuard<'a,T>
    where
        Self: 'a,
        Self::Data: 'a;

    #[inline(always)]
    fn sync_lock(&self) -> Self::Guard<'_> {
        let guard = self.mutex.lock();

        SpinMutexGuard { guard: Some(guard) }
    }

    #[inline(always)]
    fn try_sync_lock(&self) -> Option<Self::Guard<'_>> {
        self.mutex
            .try_lock()
            .map(|guard| SpinMutexGuard { guard: Some(guard) })
    }

    fn new(data: Self::Data) -> Self {
        SpinMutex {
            mutex: parking_lot::Mutex::new(data),
        }
    }
}

#[derive(Debug)]
pub struct SpinMutexGuard<'a, T> {
    guard: Option<parking_lot::MutexGuard<'a, T>>,
}

unsafe impl<'a, T> Send for SpinMutexGuard<'a, T> {}

unsafe impl<'a, T: Send> Sync for SpinMutexGuard<'a, T> {}

impl<'a, T> Drop for SpinMutexGuard<'a, T> {
    fn drop(&mut self) {
        self.unlock();
    }
}

impl<'a, T> ops::Deref for SpinMutexGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.guard.as_deref().unwrap()
    }
}

impl<'a, T> ops::DerefMut for SpinMutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard.as_deref_mut().unwrap()
    }
}

impl<'a, T> LockerGuard<'a> for SpinMutexGuard<'a, T> {
    #[inline(always)]
    fn unlock(&mut self) {
        if let Some(guard) = self.guard.take() {
            drop(guard);
        }
    }
}

use super::*;

pub type WaitableSpinMutex<T> = WaitableLockerMaker<SpinMutex<T>, SpinMutex<VecDeque<Waker>>>;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use futures::{executor::ThreadPool, task::SpawnExt};

    use crate::{
        Locker, LockerGuard, SpinMutex, WaitableLocker, WaitableLockerGuard, WaitableSpinMutex,
    };

    #[futures_test::test]
    async fn test_waitable_spin_mutex_async() {
        let pool = ThreadPool::builder().pool_size(10).create().unwrap();

        let mutex = Arc::new(WaitableSpinMutex::new(0));

        let tasks = 1000;

        let mut join_handles = vec![];

        for _ in 0..tasks {
            let mutex = mutex.clone();

            join_handles.push(
                pool.spawn_with_handle(async move {
                    let mut data = mutex.async_lock().await;

                    data.unlock();

                    for _ in 0..tasks {
                        data = data.locker().async_lock().await;
                        *data += 1;
                        data.unlock();
                    }
                })
                .unwrap(),
            );
        }

        assert_eq!(join_handles.len(), tasks);

        for handle in join_handles {
            handle.await;
        }

        assert_eq!(*mutex.async_lock().await, tasks * tasks);
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

    #[inline(never)]
    fn add_loop(shared: Arc<SpinMutex<i32>>, counter: i32) {
        for _ in 0..counter {
            let mut shared = shared.sync_lock();

            *shared += 1;
        }
    }

    #[test]
    fn test_spin_mutex() {
        let shared = Arc::new(SpinMutex::new(0));

        let threads = 8;

        let loops = 100000;

        let mut join_handles = vec![];

        for _ in 0..threads {
            let shared = shared.clone();

            join_handles.push(std::thread::spawn(move || add_loop(shared, loops)));
        }

        for handle in join_handles {
            handle.join().unwrap();
        }

        assert_eq!(*shared.sync_lock(), threads * loops);
    }
}
