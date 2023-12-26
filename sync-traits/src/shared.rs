use std::{
    cell::RefCell,
    ops,
    rc::Rc,
    sync::{Arc, Mutex, TryLockError},
};

/// A trait represents a shared value that can be dynamically checked for borrowing rules
/// or protected by mutual exclusion.
pub trait Shared {
    /// The real type of this shared value.
    type Value;

    /// The immutable reference type of this shared value.
    type Ref<'a>: ops::Deref<Target = Self::Value>
    where
        Self: 'a;

    /// The mutable reference type of this shared value.
    type RefMut<'a>: ops::DerefMut<Target = Self::Value>
    where
        Self: 'a;

    /// Lock shared value and get immutable reference.
    fn lock(&self) -> Self::Ref<'_>;

    /// Lock shared value and get mutable reference.
    fn lock_mut(&self) -> Self::RefMut<'_>;

    /// Try lock shared value and get mutable reference.
    ///
    /// If the lock is not successful, returns [`None`]
    fn try_lock_mut(&self) -> Option<Self::RefMut<'_>>;
}

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
}

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
}

impl<T> Clone for MutexShared<T> {
    fn clone(&self) -> Self {
        Self {
            value: Arc::clone(&self.value),
        }
    }
}
