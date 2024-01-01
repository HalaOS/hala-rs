use std::{collections::VecDeque, task::Waker};

use crate::{Locker, SpinMutex, WaitableLocker, WaitableLockerGuardMaker};

/// A type factory of [`WaitableLocker`].
///
/// This type combine a [`Locker`] type and [`SpinMutex<VecDeque<Waker>>`] to implement [`WaitableLocker`] trait.
pub struct WaitableLockerMaker<L>
where
    L: Locker,
{
    wakers: SpinMutex<VecDeque<Waker>>,
    inner_locker: L,
}

impl<L> Default for WaitableLockerMaker<L>
where
    L: Locker + Default,
{
    fn default() -> Self {
        Self {
            wakers: Default::default(),
            inner_locker: Default::default(),
        }
    }
}

impl<L> WaitableLocker for WaitableLockerMaker<L>
where
    L: Locker,
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

impl<L> Locker for WaitableLockerMaker<L>
where
    L: Locker,
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
