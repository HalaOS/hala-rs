#[cfg(not(feature = "single-thread"))]
use std::sync::{Arc, Mutex};
#[cfg(feature = "single-thread")]
use std::{cell::RefCell, rc::Rc};

#[derive(Debug)]
pub struct ThreadModel<T> {
    #[cfg(not(feature = "single-thread"))]
    value: Arc<Mutex<T>>,
    #[cfg(feature = "single-thread")]
    value: Rc<RefCell<T>>,
}

impl<T> Clone for ThreadModel<T> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
        }
    }
}

#[cfg(not(feature = "single-thread"))]
pub type Ref<'a, T> = std::sync::MutexGuard<'a, T>;

#[cfg(feature = "single-thread")]
pub type Ref<'a, T> = std::cell::Ref<'a, T>;

#[cfg(not(feature = "single-thread"))]
pub type RefMut<'a, T> = std::sync::MutexGuard<'a, T>;

#[cfg(feature = "single-thread")]
pub type RefMut<'a, T> = std::cell::RefMut<'a, T>;

impl<T> ThreadModel<T> {
    #[cfg(not(feature = "single-thread"))]
    pub fn new(value: T) -> Self {
        Self {
            value: Arc::new(Mutex::new(value)),
        }
    }

    #[cfg(feature = "single-thread")]
    pub fn new(value: T) -> Self {
        Self {
            value: Rc::new(RefCell::new(value)),
        }
    }

    #[cfg(not(feature = "single-thread"))]
    pub fn get(&self) -> Ref<'_, T> {
        self.value.lock().unwrap()
    }

    #[cfg(feature = "single-thread")]
    pub fn get(&self) -> Ref<'_, T> {
        self.value.borrow()
    }

    #[cfg(not(feature = "single-thread"))]
    pub fn get_mut(&self) -> RefMut<'_, T> {
        self.value.lock().unwrap()
    }

    #[cfg(feature = "single-thread")]
    pub fn get_mut(&self) -> RefMut<'_, T> {
        self.value.borrow_mut()
    }
}

impl<T> From<T> for ThreadModel<T> {
    fn from(value: T) -> Self {
        ThreadModel::new(value)
    }
}
