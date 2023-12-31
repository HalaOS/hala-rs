use std::{
    cell::UnsafeCell,
    ops,
    sync::atomic::{AtomicBool, Ordering},
};

use crate::{Locker, LockerGuard};

/// Spin mutex implementation with [`AtomicBool`]
pub struct SpinMutex<T> {
    flag: AtomicBool,
    data: UnsafeCell<T>,
}

impl<T: ?Sized + Default> Default for SpinMutex<T> {
    fn default() -> Self {
        Self {
            flag: Default::default(),
            data: Default::default(),
        }
    }
}

// these are the only places where `T: Send` matters; all other
// functionality works fine on a single thread.
unsafe impl<T: Send> Send for SpinMutex<T> {}

unsafe impl<T: Send> Sync for SpinMutex<T> {}

impl<T> SpinMutex<T> {
    /// Creates a new `SpinMutex` in an unlocked state ready for use.
    pub fn new(t: T) -> Self {
        SpinMutex {
            flag: Default::default(),
            data: UnsafeCell::new(t),
        }
    }
}

impl<T> SpinMutex<T> {
    #[cold]
    fn is_locked(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }
}

impl<T> Locker for SpinMutex<T> {
    type Data = T;

    type Guard<'a> = SpinMutexGuard<'a,T>
    where
        Self: 'a,
        Self::Data: 'a;

    #[inline(always)]
    fn sync_lock(&self) -> Self::Guard<'_> {
        while self
            .flag
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            while self.is_locked() {}
        }

        SpinMutexGuard { locker: self }
    }

    #[inline(always)]
    fn try_sync_lock(&self) -> Option<Self::Guard<'_>> {
        if self
            .flag
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            Some(SpinMutexGuard { locker: self })
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

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.locker.data.get() }
    }
}

impl<'a, T> ops::DerefMut for SpinMutexGuard<'a, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.locker.data.get() }
    }
}

impl<'a, T> LockerGuard<'a, T> for SpinMutexGuard<'a, T> {
    #[inline(always)]
    fn unlock(&mut self) {
        _ = self
            .locker
            .flag
            .compare_exchange(true, false, Ordering::Release, Ordering::Relaxed)
            .is_ok();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::{Locker, SpinMutex};

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
