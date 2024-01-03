use std::{cell::RefCell, collections::VecDeque, ops, task::Waker};

use crate::{Locker, LockerGuard, WaitableLockerMaker};

impl<T> Locker for std::cell::RefCell<T> {
    type Data = T;

    type Guard<'a> = RefcellGuard<'a, T>
    where
        Self: 'a,
        Self::Data: 'a;

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

    fn new(data: Self::Data) -> Self {
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

pub type WaitableRefCell<T> = WaitableLockerMaker<RefCell<T>, RefCell<VecDeque<Waker>>>;

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use futures::{executor::LocalPool, task::LocalSpawnExt};

    use crate::{Locker, LockerGuard, WaitableLocker, WaitableLockerGuard, WaitableRefCell};

    #[test]
    fn test_waitable_refcell_async() {
        let mut pool = LocalPool::new();

        let mutex = Rc::new(WaitableRefCell::new(0));

        let tasks = 1000;

        let mut join_handles = vec![];

        for _ in 0..tasks {
            let mutex = mutex.clone();

            join_handles.push(
                pool.spawner()
                    .spawn_local_with_handle(async move {
                        let mut data = mutex.async_lock().await;

                        data.unlock();

                        for _ in 0..tasks {
                            data = data.locker().async_lock().await;
                            *data += 1;
                            data.unlock();
                        }
                    })
                    .unwrap(),
            );
        }

        assert_eq!(join_handles.len(), tasks);

        pool.run_until(async move {
            for handle in join_handles {
                handle.await;
            }

            assert_eq!(*mutex.async_lock().await, tasks * tasks);
        });
    }
}
