use std::{cell::RefCell, ops, rc::Rc};

use crate::SharedGuardMut;

use super::{Shared, SharedGuard};

/// Shared data that using in single thread mode
#[derive(Debug, Default)]
pub struct LocalShared<T> {
    value: Rc<RefCell<T>>,
}

impl<T> Clone for LocalShared<T> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
        }
    }
}

impl<T> LocalShared<T> {
    /// Create new `LocalShared` from shared `value`.
    pub fn new(value: T) -> Self {
        value.into()
    }
}

impl<T> From<T> for LocalShared<T> {
    fn from(value: T) -> Self {
        LocalShared {
            value: Rc::new(value.into()),
        }
    }
}

impl<T> ops::Deref for LocalShared<T> {
    type Target = RefCell<T>;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T> Shared for LocalShared<T>
where
    T: Unpin,
{
    type Value = T;

    type Ref<'a> = std::cell::Ref<'a,T>
    where
        Self: 'a;

    type RefMut<'a> = std::cell::RefMut<'a,T>
    where
        Self: 'a;

    fn lock(&self) -> SharedGuard<'_, Self> {
        SharedGuard {
            value: Some(self.value.borrow()),
            shared: self,
        }
    }

    fn lock_mut(&self) -> SharedGuardMut<'_, Self> {
        SharedGuardMut {
            value: Some(self.value.borrow_mut()),
            shared: self,
        }
    }

    fn try_lock_mut(&self) -> Option<SharedGuardMut<'_, Self>> {
        match self.value.try_borrow_mut() {
            Ok(value) => Some(SharedGuardMut {
                value: Some(value),
                shared: self,
            }),
            Err(_) => None,
        }
    }

    fn try_lock(&self) -> Option<SharedGuard<'_, Self>> {
        match self.value.try_borrow() {
            Ok(value) => Some(SharedGuard {
                value: Some(value),
                shared: self,
            }),
            Err(_) => None,
        }
    }
}
