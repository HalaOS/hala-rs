use std::{
    cell::RefCell,
    ops::{Deref, DerefMut},
    rc::Rc,
    sync::{Arc, Mutex},
};

pub trait ThreadModel {
    type Holder<T>: ThreadModelHolder<T> + Clone + From<T>;
}

pub trait ThreadModelHolder<T> {
    type Ref<'a, V>: Deref<Target = V>
    where
        Self: 'a,
        V: 'a;

    type RefMut<'a, V>: DerefMut<Target = V>
    where
        Self: 'a,
        V: 'a;

    fn new(value: T) -> Self;
    /// Get immutable reference of type `T`
    fn get(&self) -> Self::Ref<'_, T>;

    /// Get mutable reference of type `T`
    fn get_mut(&self) -> Self::RefMut<'_, T>;

    fn is_multithread() -> bool;
}

/// Single thread model
pub struct STModel;

pub struct STModelHolder<T> {
    /// protected value.
    value: Rc<RefCell<T>>,
}

impl<T> Clone for STModelHolder<T> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
        }
    }
}

unsafe impl<T> Sync for STModelHolder<T> {}
unsafe impl<T> Send for STModelHolder<T> {}

impl<T> From<T> for STModelHolder<T> {
    fn from(value: T) -> Self {
        STModelHolder::new(value)
    }
}

impl<T> ThreadModelHolder<T> for STModelHolder<T> {
    type Ref<'a,V> = std::cell::Ref<'a,V> where Self:'a,V: 'a;

    type RefMut<'a,V> = std::cell::RefMut<'a,V> where Self:'a,V: 'a;

    fn new(value: T) -> Self {
        Self {
            value: Rc::new(RefCell::new(value)),
        }
    }

    fn get(&self) -> std::cell::Ref<'_, T> {
        self.value.borrow()
    }

    fn get_mut(&self) -> std::cell::RefMut<'_, T> {
        self.value.borrow_mut()
    }

    fn is_multithread() -> bool {
        false
    }
}

impl ThreadModel for STModel {
    type Holder<T> = STModelHolder<T>;
}

/// Multi thread model
pub struct MTModel;

pub struct MTModelHolder<T> {
    /// protected value.
    value: Arc<Mutex<T>>,
}

impl<T> Clone for MTModelHolder<T> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
        }
    }
}

impl<T> From<T> for MTModelHolder<T> {
    fn from(value: T) -> Self {
        MTModelHolder::new(value)
    }
}

impl<T> ThreadModelHolder<T> for MTModelHolder<T> {
    type Ref<'a,V> = std::sync::MutexGuard<'a,V> where Self:'a,V: 'a;

    type RefMut<'a,V> = std::sync::MutexGuard<'a,V> where Self:'a,V: 'a;

    fn new(value: T) -> Self {
        Self {
            value: Arc::new(Mutex::new(value)),
        }
    }

    fn get(&self) -> std::sync::MutexGuard<'_, T> {
        self.value.lock().unwrap()
    }

    fn get_mut(&self) -> std::sync::MutexGuard<'_, T> {
        self.value.lock().unwrap()
    }

    fn is_multithread() -> bool {
        true
    }
}

impl ThreadModel for MTModel {
    type Holder<T> = MTModelHolder<T>;
}
