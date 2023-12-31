use std::ops;

use crate::{Locker, LockerGuard};

impl<T> Locker for std::cell::RefCell<T> {
    type Data = T;

    type Guard<'a> = RefcellGuard<'a, T>
    where
        Self: 'a,
        Self::Data: 'a;

    fn sync_lock(&self) -> Self::Guard<'_> {
        RefcellGuard {
            locker: self,
            std_guard: Some(self.borrow_mut()),
        }
    }

    fn try_sync_lock(&self) -> Option<Self::Guard<'_>> {
        match self.try_borrow_mut() {
            Ok(guard) => Some(RefcellGuard {
                locker: self,
                std_guard: Some(guard),
            }),
            _ => None,
        }
    }
}

pub struct RefcellGuard<'a, T> {
    locker: &'a std::cell::RefCell<T>,
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

impl<'a, T> LockerGuard<'a, T> for RefcellGuard<'a, T> {
    fn unlock(&mut self) {
        self.std_guard.take();
    }

    fn sync_relock(&mut self) {
        self.std_guard = Some(self.locker.borrow_mut())
    }

    fn try_sync_relock(&mut self) -> bool {
        match self.locker.try_borrow_mut() {
            Ok(guard) => {
                self.std_guard = Some(guard);
                true
            }
            _ => false,
        }
    }
}
