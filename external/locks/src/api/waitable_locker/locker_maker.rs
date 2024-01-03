use std::{collections::VecDeque, task::Waker};

use crate::{Locker, WaitableLocker, WaitableLockerGuardMaker};

/// A type factory of [`WaitableLocker`].
///
/// This type combine a [`Locker`] type and [`SpinMutex<VecDeque<Waker>>`] to implement [`WaitableLocker`] trait.
#[derive(Debug)]
pub struct WaitableLockerMaker<L, W>
where
    L: Locker,
    W: Locker<Data = VecDeque<Waker>>,
{
    wakers: W,
    inner_locker: L,
}

impl<L, W> Default for WaitableLockerMaker<L, W>
where
    L: Locker + Default,
    W: Locker<Data = VecDeque<Waker>> + Default,
{
    fn default() -> Self {
        Self {
            wakers: Default::default(),
            inner_locker: Default::default(),
        }
    }
}

impl<L, W> WaitableLocker for WaitableLockerMaker<L, W>
where
    L: Locker,
    W: Locker<Data = VecDeque<Waker>> + Default,
{
    type WaitableGuard<'a> = WaitableLockerGuardMaker<'a,Self,L::Guard<'a>>
    where
        Self: 'a,
        Self::Data: 'a;

    fn try_lock_with_context(
        &self,
        cx: &mut std::task::Context<'_>,
    ) -> Option<Self::WaitableGuard<'_>> {
        let mut wakers = self.wakers.sync_lock();

        match self.try_sync_lock() {
            Some(guard) => Some(guard),
            None => {
                wakers.push_back(cx.waker().clone());

                None
            }
        }
    }

    fn wakeup_another_one(&self) {
        if let Some(waker) = self.wakers.sync_lock().pop_front() {
            waker.wake();
        }
    }
}

impl<L, W> Locker for WaitableLockerMaker<L, W>
where
    L: Locker,
    W: Locker<Data = VecDeque<Waker>> + Default,
{
    type Data = L::Data;

    type Guard<'a> = WaitableLockerGuardMaker<'a,Self,L::Guard<'a>>
    where
        Self: 'a,
        Self::Data: 'a;

    fn sync_lock(&self) -> Self::Guard<'_> {
        let guard = self.inner_locker.sync_lock();

        (self, guard).into()
    }

    fn try_sync_lock(&self) -> Option<Self::Guard<'_>> {
        self.inner_locker
            .try_sync_lock()
            .map(|guard| (self, guard).into())
    }

    fn new(data: Self::Data) -> Self
    where
        Self::Data: Sized,
    {
        Self {
            wakers: Default::default(),
            inner_locker: L::new(data),
        }
    }
}
