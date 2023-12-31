use std::{
    cell::UnsafeCell,
    ops,
    sync::atomic::{fence, AtomicBool, Ordering},
};

use crate::{Locker, LockerGuard};

/// Spin mutex implementation with [`AtomicBool`]
pub struct SpinMutex<T: ?Sized> {
    flag: AtomicBool,
    data: UnsafeCell<T>,
}

impl<T: ?Sized + Default> Default for SpinMutex<T> {
    fn default() -> Self {
        Self {
            flag: Default::default(),
            data: UnsafeCell::default(),
        }
    }
}

// these are the only places where `T: Send` matters; all other
// functionality works fine on a single thread.
unsafe impl<T: ?Sized + Send> Send for SpinMutex<T> {}

unsafe impl<T: ?Sized + Send> Sync for SpinMutex<T> {}

impl<T> SpinMutex<T> {
    /// Creates a new `SpinMutex` in an unlocked state ready for use.
    pub fn new(t: T) -> Self {
        SpinMutex {
            flag: Default::default(),
            data: UnsafeCell::new(t),
        }
    }
}

impl<T> Locker for SpinMutex<T> {
    type Data = T;

    type Guard<'a> = SpinMutexGuard<'a,T>
    where
        Self: 'a,
        Self::Data: 'a;

    #[inline]
    fn sync_lock(&self) -> Self::Guard<'_> {
        let mut guard = SpinMutexGuard { locker: self };

        guard.sync_relock();

        guard
    }

    #[inline]
    fn try_sync_lock(&self) -> Option<Self::Guard<'_>> {
        let mut guard = SpinMutexGuard { locker: self };

        if guard.try_sync_relock() {
            Some(guard)
        } else {
            None
        }
    }
}

pub struct SpinMutexGuard<'a, T> {
    locker: &'a SpinMutex<T>,
}

impl<'a, T> Drop for SpinMutexGuard<'a, T> {
    fn drop(&mut self) {
        self.unlock();
    }
}

impl<'a, T> ops::Deref for SpinMutexGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.locker.data.get() }
    }
}

impl<'a, T> ops::DerefMut for SpinMutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.locker.data.get() }
    }
}

impl<'a, T> LockerGuard<'a, T> for SpinMutexGuard<'a, T> {
    #[inline]
    fn unlock(&mut self) {
        self.locker.flag.store(false, Ordering::Release);
    }

    #[inline]
    fn sync_relock(&mut self) {
        while self
            .locker
            .flag
            .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {}

        fence(Ordering::Acquire);
    }

    #[inline]
    fn try_sync_relock(&mut self) -> bool {
        if self
            .locker
            .flag
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::{Locker, SpinMutex};

    #[test]
    fn test_spin_mutex() {
        let shared = Arc::new(SpinMutex::new(0));

        let threads = 8;

        let loops = 1000;

        let mut join_handles = vec![];

        for _ in 0..threads {
            let shared = shared.clone();

            join_handles.push(std::thread::spawn(move || {
                for _ in 0..loops {
                    let mut shared = shared.sync_lock();

                    *shared += 1;
                }
            }));
        }

        for handle in join_handles {
            handle.join().unwrap();
        }

        assert_eq!(*shared.sync_lock(), threads * loops);
    }
}
