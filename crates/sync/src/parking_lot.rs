use std::ops;

use crate::{maker::*, Lockable, LockableNew};

/// A spin style mutex implementation without handle thread-specific data.
pub struct SpinMutex<T> {
    mutex: parking_lot::Mutex<T>,
}

impl<T> LockableNew for SpinMutex<T> {
    type Value = T;

    /// Creates a new mutex in an unlocked state ready for use.
    fn new(t: T) -> Self {
        Self {
            mutex: parking_lot::Mutex::new(t),
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

    #[inline]
    fn lock(&self) -> Self::GuardMut<'_> {
        SpinMutexGuard {
            locker: self,
            guard: Some(self.mutex.lock()),
        }
    }

    #[inline]
    fn try_lock(&self) -> Option<Self::GuardMut<'_>> {
        if let Some(guard) = self.mutex.try_lock() {
            Some(SpinMutexGuard {
                locker: self,
                guard: Some(guard),
            })
        } else {
            None
        }
    }

    #[inline]
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
    guard: Option<parking_lot::MutexGuard<'a, T>>,
}

impl<'a, T> Drop for SpinMutexGuard<'a, T> {
    fn drop(&mut self) {
        drop(self.guard.take())
    }
}

impl<'a, T> ops::Deref for SpinMutexGuard<'a, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.guard.as_deref().unwrap()
    }
}

impl<'a, T> ops::DerefMut for SpinMutexGuard<'a, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard.as_deref_mut().unwrap()
    }
}

unsafe impl<'a, T: Send> Send for SpinMutexGuard<'a, T> {}

/// Futures-aware [`SpinMutex`] type
pub type AsyncSpinMutex<T> =
    AsyncLockableMaker<SpinMutex<T>, SpinMutex<DefaultAsyncLockableMediator>>;
