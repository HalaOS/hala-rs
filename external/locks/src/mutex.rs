use std::{collections::VecDeque, ops, task::Waker};

use crate::{Locker, LockerGuard, SpinMutex, WaitableLockerMaker};

impl<T> Locker for std::sync::Mutex<T> {
    type Data = T;

    type Guard<'a> = MutexGuard<'a, T>
    where
        Self: 'a,
        Self::Data: 'a;

    fn sync_lock(&self) -> Self::Guard<'_> {
        MutexGuard {
            std_guard: Some(self.lock().unwrap()),
        }
    }

    fn try_sync_lock(&self) -> Option<Self::Guard<'_>> {
        match self.try_lock() {
            Ok(guard) => Some(MutexGuard {
                std_guard: Some(guard),
            }),
            _ => None,
        }
    }

    fn new(data: Self::Data) -> Self {
        std::sync::Mutex::new(data)
    }
}

#[derive(Debug)]
pub struct MutexGuard<'a, T> {
    std_guard: Option<std::sync::MutexGuard<'a, T>>,
}

impl<'a, T> ops::Deref for MutexGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.std_guard.as_deref().unwrap()
    }
}

impl<'a, T> ops::DerefMut for MutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.std_guard.as_deref_mut().unwrap()
    }
}

impl<'a, T> LockerGuard<'a> for MutexGuard<'a, T> {
    fn unlock(&mut self) {
        self.std_guard.take();
    }
}

pub type WaitableMutex<T> = WaitableLockerMaker<std::sync::Mutex<T>, SpinMutex<VecDeque<Waker>>>;

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
