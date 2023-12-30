use std::sync::TryLockError;

use super::*;

/// The type extend std [`Mutex`](std::sync::Mutex) type to support the `Locker`,`WaitableLocker` traits.
pub struct Mutex<T: ?Sized> {
    wakers: Arc<lockfree::Queue<Waker>>,
    std_mutex: std::sync::Mutex<T>,
}

impl<T> Mutex<T> {
    /// Creates a new mutex in an unlocked state ready for use.
    pub fn new(t: T) -> Self {
        Self {
            wakers: Default::default(),
            std_mutex: std::sync::Mutex::new(t),
        }
    }
}

impl<T> Locker for Mutex<T>
where
    T: Unpin,
{
    type Data = T;

    type Guard<'a> = MutexWaitableLockerGuard<'a,T>
    where
        Self: 'a,
        Self::Data: 'a;

    fn lock(&self) -> Self::Guard<'_> {
        let std_guard = self.std_mutex.lock().unwrap();

        MutexWaitableLockerGuard {
            std_guard: Some(std_guard),
            mutex: self,
        }
    }

    fn try_lock(&self) -> Option<Self::Guard<'_>> {
        match self.std_mutex.try_lock() {
            Ok(guard) => Some(MutexWaitableLockerGuard {
                std_guard: Some(guard),
                mutex: self,
            }),
            Err(TryLockError::Poisoned(err)) => Some(MutexWaitableLockerGuard {
                std_guard: Some(err.into_inner()),
                mutex: self,
            }),
            Err(_) => None,
        }
    }
}

impl<T> WaitableLocker for Mutex<T>
where
    T: Unpin,
{
    type WaitableGuard<'a>= MutexWaitableLockerGuard<'a,T>
    where
        Self: 'a,
        Self::Data: 'a;

    fn try_lock_with_context(&self, cx: &mut Context<'_>) -> Option<Self::WaitableGuard<'_>> {
        match self.try_lock() {
            Some(guard) => Some(guard),
            None => {
                self.wakers.push(cx.waker().clone());

                // try again to skip data race.
                self.try_lock()
            }
        }
    }
}

pub struct MutexWaitableLockerGuard<'a, T: ?Sized + 'a> {
    std_guard: Option<std::sync::MutexGuard<'a, T>>,
    mutex: &'a Mutex<T>,
}

impl<'a, T: ?Sized + 'a> Drop for MutexWaitableLockerGuard<'a, T> {
    fn drop(&mut self) {
        if let Some(guard) = self.std_guard.take() {
            drop(guard);

            if let Some(waker) = self.mutex.wakers.pop() {
                waker.wake();
            }
        }
    }
}

impl<'a, T: ?Sized + 'a> ops::Deref for MutexWaitableLockerGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.std_guard.as_deref().unwrap()
    }
}

impl<'a, T: ?Sized + 'a> ops::DerefMut for MutexWaitableLockerGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.std_guard.as_deref_mut().unwrap()
    }
}

impl<'a, T: ?Sized + 'a> LockerGuard<'a, T> for MutexWaitableLockerGuard<'a, T> {
    fn unlock(&mut self) {
        drop(self.std_guard.take().expect("Call unlock twice"));

        if let Some(waker) = self.mutex.wakers.pop() {
            waker.wake();
        }
    }

    fn relock(&mut self) {
        let std_guard = match self.mutex.std_mutex.lock() {
            Ok(guard) => guard,
            Err(err) => err.into_inner(),
        };

        self.std_guard = Some(std_guard);
    }

    fn try_relock(&mut self) -> bool {
        let std_guard = match self.mutex.std_mutex.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),

            Err(_) => return false,
        };

        self.std_guard = Some(std_guard);

        return true;
    }
}

impl<'a, T> WaitableLockerGuard<'a, T> for MutexWaitableLockerGuard<'a, T>
where
    T: Unpin + 'a,
{
    type Locker = Mutex<T>;

    fn inner(&self) -> &'a Self::Locker {
        self.mutex
    }
}
