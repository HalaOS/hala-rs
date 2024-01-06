use std::{cell::RefCell, collections::VecDeque, ops, task::Waker};

use crate::{Locker, LockerGuard, WaitableLockerMaker};

impl<T> Locker for std::cell::RefCell<T> {
    type Value = T;

    type Guard<'a> = RefcellGuard<'a, T>
    where
        Self: 'a,
        Self::Value: 'a;

    fn sync_lock(&self) -> Self::Guard<'_> {
        RefcellGuard {
            std_guard: Some(self.borrow_mut()),
        }
    }

    fn try_sync_lock(&self) -> Option<Self::Guard<'_>> {
        match self.try_borrow_mut() {
            Ok(guard) => Some(RefcellGuard {
                std_guard: Some(guard),
            }),
            _ => None,
        }
    }

    fn new(data: Self::Value) -> Self {
        std::cell::RefCell::new(data)
    }
}

#[derive(Debug)]
pub struct RefcellGuard<'a, T> {
    std_guard: Option<std::cell::RefMut<'a, T>>,
}

impl<'a, T> ops::Deref for RefcellGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.std_guard.as_deref().unwrap()
    }
}

impl<'a, T> ops::DerefMut for RefcellGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.std_guard.as_deref_mut().unwrap()
    }
}

impl<'a, T> LockerGuard<'a> for RefcellGuard<'a, T> {
    fn unlock(&mut self) {
        self.std_guard.take();
    }
}
