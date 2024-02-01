use std::ffi::c_int;

extern "C" {
    /// Reentrancy guard counter plus 1.
    fn reentrancy_guard_counter_add() -> c_int;

    /// Reentrancy guard counter sub 1.
    fn reentrancy_guard_counter_sub() -> c_int;

    /// locks the backtrace mutex, blocks if the mutex is not available
    fn backtrace_mutex_lock();

    /// unlocks the backtrace mutex.
    fn backtrace_mutex_unlock();

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
