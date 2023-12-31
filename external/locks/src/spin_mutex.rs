use std::ops;

use crate::{Locker, LockerGuard};

/// Spin mutex implementation with [`AtomicBool`]
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
unsafe impl<T: Send> Send for SpinMutex<T> {}

unsafe impl<T: Send> Sync for SpinMutex<T> {}

impl<T> SpinMutex<T> {
    /// Creates a new `SpinMutex` in an unlocked state ready for use.
    pub fn new(t: T) -> Self {
        SpinMutex {
            mutex: parking_lot::Mutex::new(t),
        }
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
        let mut guard = SpinMutexGuard {
            locker: self,
            guard: None,
        };

        guard.sync_relock();

        guard
    }

    #[inline(always)]
    fn try_sync_lock(&self) -> Option<Self::Guard<'_>> {
        let mut guard = SpinMutexGuard {
            locker: self,
            guard: None,
        };

        if guard.try_sync_relock() {
            Some(guard)
        } else {
            None
        }
    }
}

pub struct SpinMutexGuard<'a, T> {
    locker: &'a SpinMutex<T>,
    guard: Option<parking_lot::MutexGuard<'a, T>>,
}

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

impl<'a, T> LockerGuard<'a, T> for SpinMutexGuard<'a, T> {
    #[inline(always)]
    fn unlock(&mut self) {
        if let Some(guard) = self.guard.take() {
            drop(guard);
        }
    }

    #[inline(always)]
    fn sync_relock(&mut self) {
        self.guard = Some(self.locker.mutex.lock())
    }

    #[inline(always)]
    fn try_sync_relock(&mut self) -> bool {
        match self.locker.mutex.try_lock() {
            None => false,
            guard => {
                self.guard = guard;
                true
            }
        }
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
