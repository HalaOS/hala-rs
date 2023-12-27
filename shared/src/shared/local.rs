use std::{cell::RefCell, ops, rc::Rc};

use super::Shared;

impl<T> Shared for RefCell<T> {
    type Value = T;

    type Ref<'a> = std::cell::Ref<'a,T>
    where
        Self: 'a;

    type RefMut<'a> = std::cell::RefMut<'a,T>
    where
        Self: 'a;

    fn lock(&self) -> Self::Ref<'_> {
        self.borrow()
    }

    fn lock_mut(&self) -> Self::RefMut<'_> {
        self.borrow_mut()
    }

    fn try_lock_mut(&self) -> Option<Self::RefMut<'_>> {
        match self.try_borrow_mut() {
            Ok(value) => Some(value),
            // the value is currently borrowed
            _ => None,
        }
    }

    fn try_lock(&self) -> Option<Self::Ref<'_>> {
        match self.try_borrow() {
            Ok(value) => Some(value),
            // the value is currently borrowed
            _ => None,
        }
    }
}

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

impl<T> Shared for LocalShared<T> {
    type Value = T;

    type Ref<'a> = std::cell::Ref<'a,T>
    where
        Self: 'a;

    type RefMut<'a> = std::cell::RefMut<'a,T>
    where
        Self: 'a;

    fn lock(&self) -> Self::Ref<'_> {
        self.value.lock()
    }

    fn lock_mut(&self) -> Self::RefMut<'_> {
        self.value.lock_mut()
    }

    fn try_lock_mut(&self) -> Option<Self::RefMut<'_>> {
        self.value.try_lock_mut()
    }

    fn try_lock(&self) -> Option<Self::Ref<'_>> {
        self.value.try_lock()
    }
}
