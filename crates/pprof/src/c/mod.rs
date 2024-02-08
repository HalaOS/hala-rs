use std::{
    cell::UnsafeCell,
    ffi::c_int,
    ops::{Deref, DerefMut},
    os::raw::c_void,
};

extern "C" {
    /// Reentrancy guard counter plus 1.
    fn reentrancy_guard_counter_add() -> c_int;

    /// Reentrancy guard counter sub 1.
    fn reentrancy_guard_counter_sub() -> c_int;

    /// locks the backtrace mutex, blocks if the mutex is not available
    fn backtrace_mutex_lock();

    /// unlocks the backtrace mutex.
    fn backtrace_mutex_unlock();

    /// Create a new cpp recursive_mutex.
    fn recursive_mutex_create() -> *const c_void;

    /// destroy the cpp recursive_mutex.
    fn recursive_mutex_destroy(mutex: *const c_void);

    /// lock recursive_mutex.
    fn recursive_mutex_lock(mutex: *const c_void);

    /// unlock recursive_mutex.
    fn recursive_mutex_unlock(mutex: *const c_void);

}

pub(crate) struct RecursiveMutex<T> {
    c_mutex: *const c_void,
    cell: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for RecursiveMutex<T> {}
unsafe impl<T: Send> Sync for RecursiveMutex<T> {}

impl<T> Default for RecursiveMutex<T>
where
    T: Default,
{
    fn default() -> Self {
        Self::new(Default::default())
    }
}

impl<T> RecursiveMutex<T> {
    pub fn new(v: T) -> Self {
        Self {
            c_mutex: unsafe { recursive_mutex_create() },
            cell: UnsafeCell::new(v),
        }
    }

    pub fn lock(&self) -> RecursiveMutexGuard<'_, T> {
        unsafe {
            recursive_mutex_lock(self.c_mutex);
        }

        RecursiveMutexGuard { mutex: self }
    }
}

impl<T> Drop for RecursiveMutex<T> {
    fn drop(&mut self) {
        unsafe { recursive_mutex_destroy(self.c_mutex) }
    }
}

pub(crate) struct RecursiveMutexGuard<'a, T> {
    mutex: &'a RecursiveMutex<T>,
}

impl<'a, T> Drop for RecursiveMutexGuard<'a, T> {
    fn drop(&mut self) {
        unsafe {
            recursive_mutex_unlock(self.mutex.c_mutex);
        }
    }
}

impl<'a, T> Deref for RecursiveMutexGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.mutex.cell.get() }
    }
}

impl<'a, T> DerefMut for RecursiveMutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.mutex.cell.get() }
    }
}

/// Reentrancy guard
pub(crate) struct Reentrancy(c_int);

impl Reentrancy {
    /// Create new reentrancy guard.
    #[inline]
    pub(crate) fn new() -> Self {
        Self(unsafe { reentrancy_guard_counter_add() })
    }
}

impl Reentrancy {
    /// Return true if first enter the scope.
    #[inline]
    pub(crate) fn is_ok(&self) -> bool {
        self.0 == 1
    }
}

impl Drop for Reentrancy {
    #[inline]
    fn drop(&mut self) {
        unsafe {
            reentrancy_guard_counter_sub();
        }
    }
}

/// Backtrace mod mutex guard.
pub(crate) struct BacktraceGuard;

/// Synchronize backtrace api calls and returns `locker` guard.
#[inline]
pub(crate) fn backtrace_lock() -> BacktraceGuard {
    unsafe {
        backtrace_mutex_lock();
    }

    BacktraceGuard
}

impl Drop for BacktraceGuard {
    #[inline]
    fn drop(&mut self) {
        unsafe { backtrace_mutex_unlock() }
    }
}
