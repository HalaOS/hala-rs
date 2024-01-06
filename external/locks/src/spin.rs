use std::{
    cell::UnsafeCell,
    collections::VecDeque,
    ops,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    task::Waker,
};

use crate::{maker::AsyncLockableMaker, Lockable, LockableNew};

/// A spin style mutex implementation without handle thread-specific data.
pub struct SpinMutex<T> {
    /// The lock status flag.
    flag: AtomicBool,
    /// Pointer to the Guard object that owns the lock, or 0 if no Guard object owns the lock.
    guard: AtomicUsize,
    /// unsafe cell to hold protected data.
    data: UnsafeCell<T>,
}

impl<T> LockableNew for SpinMutex<T> {
    type Value = T;

    /// Creates a new mutex in an unlocked state ready for use.
    fn new(t: T) -> Self {
        Self {
            flag: AtomicBool::new(false),
            data: t.into(),
            guard: AtomicUsize::new(0),
        }
    }
}

impl<T: Default> Default for SpinMutex<T> {
    fn default() -> Self {
        Self::new(Default::default())
    }
}

impl<T> Lockable for SpinMutex<T> {
    type GuardMut<'a> = SpinMutexGuard<'a, T>
    where
        Self: 'a;

    fn lock(&self) -> Self::GuardMut<'_> {
        while self
            .flag
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {}

        let mut guard = SpinMutexGuard {
            locker: self,
            ptr: 0,
        };

        guard.ptr = &guard as *const _ as usize;

        self.guard
            .compare_exchange(
                0,
                &guard as *const _ as usize,
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .expect("Set guard ptr error");

        guard
    }

    fn try_lock(&self) -> Option<Self::GuardMut<'_>> {
        if self
            .flag
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            let mut guard = SpinMutexGuard {
                locker: self,
                ptr: 0,
            };

            guard.ptr = &guard as *const _ as usize;

            self.guard
                .compare_exchange(
                    0,
                    &guard as *const _ as usize,
                    Ordering::Acquire,
                    Ordering::Relaxed,
                )
                .expect("Set guard ptr error");

            Some(guard)
        } else {
            None
        }
    }

    fn unlock(guard: Self::GuardMut<'_>) -> &Self {
        let locker = guard.locker;

        drop(guard);

        locker
    }
}

/// RAII type that handle `scope lock` semantics
pub struct SpinMutexGuard<'a, T> {
    /// a reference to the associated [`SpinMutex`]
    locker: &'a SpinMutex<T>,
    ptr: usize,
}

impl<'a, T> Drop for SpinMutexGuard<'a, T> {
    fn drop(&mut self) {
        self.locker
            .guard
            .compare_exchange(self.ptr, 0, Ordering::Release, Ordering::Relaxed)
            .expect("Unset guard ptr error");

        self.locker.flag.store(false, Ordering::Release);
    }
}

impl<'a, T> SpinMutexGuard<'a, T> {
    fn deref_check(&self) {
        assert_eq!(
            self.locker.guard.load(Ordering::Acquire),
            self.ptr,
            "fail to check constraint of deref/deref_mut ops"
        );
    }
}

impl<'a, T> ops::Deref for SpinMutexGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.deref_check();
        unsafe { &*self.locker.data.get() }
    }
}

impl<'a, T> ops::DerefMut for SpinMutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.deref_check();
        unsafe { &mut *self.locker.data.get() }
    }
}

// these are the only places where `T: Send` matters; all other
// functionality works fine on a single thread.
unsafe impl<T: Send> Send for SpinMutex<T> {}
unsafe impl<T: Send> Sync for SpinMutex<T> {}

// Safe to send since we don't track any thread-specific details
unsafe impl<'a, T: Send> Send for SpinMutexGuard<'a, T> {}
unsafe impl<'a, T: Sync> Sync for SpinMutexGuard<'a, T> {}

/// Futures-aware [`SpinMutex`] type
pub type AsyncSpinMutex<T> = AsyncLockableMaker<SpinMutex<T>, SpinMutex<VecDeque<Waker>>>;

#[cfg(test)]
mod tests {
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };

    use futures::{executor::ThreadPool, task::SpawnExt};

    use crate::{AsyncLockable, AsyncSpinMutex};

    #[futures_test::test]
    async fn test_async_lock() {
        let loops = 1000;

        let pool = ThreadPool::builder().pool_size(10).create().unwrap();

        let shared = Arc::new(AsyncSpinMutex::new(0));

        let mut join_handles = vec![];

        for _ in 0..loops {
            let shared = shared.clone();

            join_handles.push(
                pool.spawn_with_handle(async move {
                    let mut data = shared.lock().await;

                    AsyncSpinMutex::unlock(data);

                    for _ in 0..loops {
                        data = shared.lock().await;

                        *data += 1;

                        AsyncSpinMutex::unlock(data);
                    }
                })
                .unwrap(),
            );
        }

        for join in join_handles {
            join.await
        }

        assert_eq!(*shared.lock().await, loops * loops);
    }

    #[futures_test::test]
    async fn bench_async_lock() {
        let loops = 1000000;

        let shared = AsyncSpinMutex::new(0);

        let mut duration = Duration::from_secs(0);

        for _ in 0..loops {
            let start = Instant::now();
            let mut shared = shared.lock().await;
            duration += start.elapsed();

            *shared += 1;
        }

        assert_eq!(*shared.lock().await, loops);

        println!("bench_async_lock: {:?}", duration / loops);
    }
}
