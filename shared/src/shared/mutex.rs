use std::{
    ops,
    sync::{Arc, Mutex, TryLockError},
};

use super::Shared;

impl<T> Shared for std::sync::Mutex<T> {
    type Value = T;

    type Ref<'a> = std::sync::MutexGuard<'a,T>
    where
        Self: 'a;

    type RefMut<'a> = std::sync::MutexGuard<'a,T>
    where
        Self: 'a;

    fn lock(&self) -> Self::Ref<'_> {
        self.lock().unwrap()
    }

    fn lock_mut(&self) -> Self::RefMut<'_> {
        self.lock().unwrap()
    }

    fn try_lock_mut(&self) -> Option<Self::RefMut<'_>> {
        match self.try_lock() {
            Ok(value) => Some(value),
            // the value is currently borrowed
            Err(err) => {
                log::trace!("{}", err);

                if let TryLockError::Poisoned(_) = err {
                    panic!("Poisoned");
                }

                None
            }
        }
    }

    fn try_lock(&self) -> Option<Self::Ref<'_>> {
        match self.try_lock() {
            Ok(value) => Some(value),
            // the value is currently borrowed
            Err(err) => {
                log::trace!("{}", err);

                if let TryLockError::Poisoned(_) = err {
                    panic!("Poisoned");
                }

                None
            }
        }
    }
}

/// Shared data that using in multi-thread mode
#[derive(Debug)]
pub struct MutexShared<T> {
    value: Arc<Mutex<T>>,
}

impl<T> Default for MutexShared<T>
where
    T: Default,
{
    fn default() -> Self {
        Self {
            value: Arc::new(Mutex::default()),
        }
    }
}

impl<T> ops::Deref for MutexShared<T> {
    type Target = Mutex<T>;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T> From<T> for MutexShared<T> {
    fn from(value: T) -> Self {
        MutexShared {
            value: Arc::new(value.into()),
        }
    }
}

impl<T> MutexShared<T> {
    /// Create new `MutexShared` from shared `value`.
    pub fn new(value: T) -> Self {
        value.into()
    }
}

impl<T> Shared for MutexShared<T> {
    type Value = T;

    type Ref<'a> = std::sync::MutexGuard<'a,T>
    where
        Self: 'a;

    type RefMut<'a> = std::sync::MutexGuard<'a,T>
    where
        Self: 'a;

    fn lock(&self) -> Self::Ref<'_> {
        self.value.lock().unwrap()
    }

    fn lock_mut(&self) -> Self::RefMut<'_> {
        self.value.lock_mut()
    }

    fn try_lock_mut(&self) -> Option<Self::RefMut<'_>> {
        self.value.try_lock_mut()
    }

    fn try_lock(&self) -> Option<Self::Ref<'_>> {
        match self.value.try_lock() {
            Ok(value) => Some(value),
            // the value is currently borrowed
            Err(err) => {
                log::trace!("{}", err);

                if let TryLockError::Poisoned(_) = err {
                    panic!("Poisoned");
                }

                None
            }
        }
    }
}

impl<T> Clone for MutexShared<T> {
    fn clone(&self) -> Self {
        Self {
            value: Arc::clone(&self.value),
        }
    }
}
