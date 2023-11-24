#[cfg(feature = "multi-thread")]
use std::sync::{Arc, Mutex};
#[cfg(not(feature = "multi-thread"))]
use std::{cell::RefCell, rc::Rc};

#[derive(Debug)]
pub struct ThreadModel<T> {
    #[cfg(feature = "multi-thread")]
    value: Arc<Mutex<T>>,
    #[cfg(not(feature = "multi-thread"))]
    value: Rc<RefCell<T>>,
}

#[cfg(not(feature = "multi-thread"))]
unsafe impl<T> Send for ThreadModel<T> {}

#[cfg(not(feature = "multi-thread"))]
unsafe impl<T> Sync for ThreadModel<T> {}

impl<T> Clone for ThreadModel<T> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
        }
    }
}

#[cfg(feature = "multi-thread")]
pub type Ref<'a, T> = std::sync::MutexGuard<'a, T>;

#[cfg(not(feature = "multi-thread"))]
pub type Ref<'a, T> = std::cell::Ref<'a, T>;

#[cfg(feature = "multi-thread")]
pub type RefMut<'a, T> = std::sync::MutexGuard<'a, T>;

#[cfg(not(feature = "multi-thread"))]
pub type RefMut<'a, T> = std::cell::RefMut<'a, T>;

impl<T> ThreadModel<T> {
    #[cfg(feature = "multi-thread")]
    pub fn new(value: T) -> Self {
        log::trace!("multi-thread ThreadModel");
        Self {
            value: Arc::new(Mutex::new(value)),
        }
    }

    #[cfg(not(feature = "multi-thread"))]
    pub fn new(value: T) -> Self {
        log::trace!("single-thread ThreadModel");
        Self {
            value: Rc::new(RefCell::new(value)),
        }
    }

    #[cfg(feature = "multi-thread")]
    pub fn get(&self) -> Ref<'_, T> {
        self.value.lock().unwrap()
    }

    #[cfg(not(feature = "multi-thread"))]
    pub fn get(&self) -> Ref<'_, T> {
        self.value.borrow()
    }

    #[cfg(feature = "multi-thread")]
    pub fn get_mut(&self) -> RefMut<'_, T> {
        self.value.lock().unwrap()
    }

    #[cfg(not(feature = "multi-thread"))]
    pub fn get_mut(&self) -> RefMut<'_, T> {
        self.value.borrow_mut()
    }
}

impl<T> From<T> for ThreadModel<T> {
    fn from(value: T) -> Self {
        ThreadModel::new(value)
    }
}
