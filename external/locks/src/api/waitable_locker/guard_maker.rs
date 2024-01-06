use std::ops;

use super::*;

/// The type factory of [`WaitableLockerGuard`] type
#[derive(Debug)]
pub struct WaitableLockerGuardMaker<'a, L, G>
where
    L: WaitableLocker<WaitableGuard<'a> = Self>,
    G: Send + 'a,
{
    /// The [`Locker`] instance associated with this guard
    locker: &'a L,
    /// Sync version guard of filed [`locker`](Locker)
    guard: Option<G>,
}

impl<'a, L, G> From<(&'a L, G)> for WaitableLockerGuardMaker<'a, L, G>
where
    L: WaitableLocker<WaitableGuard<'a> = Self>,
    G: Send + 'a,
{
    fn from(value: (&'a L, G)) -> Self {
        Self {
            locker: value.0,
            guard: Some(value.1),
        }
    }
}

impl<'a, L, G> WaitableLockerGuard<'a> for WaitableLockerGuardMaker<'a, L, G>
where
    L: WaitableLocker<WaitableGuard<'a> = Self>,
    G: Send + 'a,
{
    type Locker = L
    where
        Self: 'a;

    fn locker(&self) -> &'a Self::Locker {
        self.locker
    }

    fn unlock(&mut self) {
        if let Some(guard) = self.guard.take() {
            drop(guard);

            self.locker.wakeup_another_one();
        }
    }
}

impl<'a, L, G> Drop for WaitableLockerGuardMaker<'a, L, G>
where
    L: WaitableLocker<WaitableGuard<'a> = Self>,
    G: Send + 'a,
{
    fn drop(&mut self) {
        self.unlock();
    }
}

impl<'a, L, G> ops::Deref for WaitableLockerGuardMaker<'a, L, G>
where
    L: WaitableLocker<WaitableGuard<'a> = Self>,
    G: Send + 'a + ops::Deref,
{
    type Target = G::Target;
    fn deref(&self) -> &Self::Target {
        self.guard.as_deref().expect("Deref on unlocked guard")
    }
}

impl<'a, L, G> ops::DerefMut for WaitableLockerGuardMaker<'a, L, G>
where
    L: WaitableLocker<WaitableGuard<'a> = Self>,
    G: Send + 'a + ops::DerefMut,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard
            .as_deref_mut()
            .expect("DerefMut on unlocked guard")
    }
}
