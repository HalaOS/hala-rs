use std::{
    collections::VecDeque,
    task::{Poll, Waker},
};

use crate::{Locker, WaitableLocker, WaitableLockerGuardMaker};

/// A type factory of [`WaitableLocker`].
///
/// This type combine a [`Locker`] type and [`SpinMutex<VecDeque<Waker>>`] to implement [`WaitableLocker`] trait.
#[derive(Debug)]
pub struct WaitableLockerMaker<L, W>
where
    L: Locker,
    W: Locker<Value = VecDeque<Waker>>,
{
    wakers: W,
    inner_locker: L,
}

impl<L, W> Default for WaitableLockerMaker<L, W>
where
    L: Locker + Default,
    W: Locker<Value = VecDeque<Waker>> + Default,
{
    fn default() -> Self {
        Self {
            wakers: Default::default(),
            inner_locker: Default::default(),
        }
    }
}

impl<L, W> WaitableLockerMaker<L, W>
where
    L: Locker,
    W: Locker<Value = VecDeque<Waker>> + Default,
{
    pub fn new(value: L::Value) -> Self
    where
        L::Value: Sized,
    {
        Self {
            wakers: Default::default(),
            inner_locker: L::new(value),
        }
    }
}

impl<L, W> WaitableLocker for WaitableLockerMaker<L, W>
where
    L: Locker,
    for<'a> L::Guard<'a>: Send + 'a,
    W: Locker<Value = VecDeque<Waker>> + Default,
{
    type WaitableGuard<'a> = WaitableLockerGuardMaker<'a,Self,L::Guard<'a>>
    where
        Self: 'a;

    fn poll_lock(&self, cx: &mut std::task::Context<'_>) -> Poll<Self::WaitableGuard<'_>> {
        let mut wakers = self.wakers.sync_lock();

        match self.inner_locker.try_sync_lock() {
            Some(guard) => Poll::Ready((self, guard).into()),
            None => {
                wakers.push_back(cx.waker().clone());

                Poll::Pending
            }
        }
    }

    fn wakeup_another_one(&self) {
        if let Some(waker) = self.wakers.sync_lock().pop_front() {
            waker.wake();
        }
    }
}
