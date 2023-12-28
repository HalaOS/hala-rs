use std::{
    ops,
    sync::{Arc, Mutex},
};

use crate::{SharedGuard, SharedGuardMut};

use super::Shared;

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

impl<T> Shared for MutexShared<T>
where
    T: Unpin,
{
    type Value = T;

    type Ref<'a> = std::sync::MutexGuard<'a,T>
    where
        Self: 'a;

    type RefMut<'a> = std::sync::MutexGuard<'a,T>
    where
        Self: 'a;

    fn lock(&self) -> SharedGuard<'_, Self> {
        SharedGuard {
            value: Some(self.value.lock().unwrap()),
            shared: self,
        }
    }

    fn lock_mut(&self) -> SharedGuardMut<'_, Self> {
        SharedGuardMut {
            value: Some(self.value.lock().unwrap()),
            shared: self,
        }
    }

    fn try_lock_mut(&self) -> Option<SharedGuardMut<'_, Self>> {
        match self.value.try_lock() {
            Ok(value) => Some(SharedGuardMut {
                value: Some(value),
                shared: self,
            }),
            Err(_) => None,
        }
    }

    fn try_lock(&self) -> Option<SharedGuard<'_, Self>> {
        match self.value.try_lock() {
            Ok(value) => Some(SharedGuard {
                value: Some(value),
                shared: self,
            }),
            Err(_) => None,
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
