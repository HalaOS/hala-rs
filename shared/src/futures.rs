use std::{future::Future, ops};

use crate::Shared;

/// `Shared` extension trait for asynchronization program.
pub trait AsyncShared: Shared {
    /// Future type created by [`lock_wait`](AsyncShared::lock_wait)
    type RefFuture<'a>: Future<Output = AsyncSharedGuard<'a, Self>>
    where
        Self: 'a;

    /// Future type created by [`lock_mut_wait`](AsyncShared::lock_wait)
    type RefMutFuture<'a>: Future<Output = AsyncSharedGuardMut<'a, Self>>
    where
        Self: 'a;

    /// Create new future and wait locking shared value and getting immutable reference.
    fn lock_wait(&self) -> Self::RefFuture<'_>;

    /// Create new future and wait locking shared value and getting mutable reference.
    fn lock_mut_wait(&self) -> Self::RefMutFuture<'_>;
}

pub struct AsyncSharedGuard<'a, S>
where
    S: AsyncShared + ?Sized,
{
    pub value: Option<S::Ref<'a>>,
    pub shared: &'a S,
}

unsafe impl<'a, S> Send for AsyncSharedGuard<'a, S> where S: AsyncShared + ?Sized {}

impl<'a, S> ops::Deref for AsyncSharedGuard<'a, S>
where
    S: AsyncShared,
{
    type Target = S::Value;

    fn deref(&self) -> &Self::Target {
        &self.value.as_ref().expect("unlocked")
    }
}

impl<'a, S> AsyncSharedGuard<'a, S>
where
    S: AsyncShared + ?Sized,
{
    pub fn unlock(&mut self) {
        self.value.take();
    }

    pub async fn relock(&mut self) {
        self.value = self.shared.lock_wait().await.value
    }
}

pub struct AsyncSharedGuardMut<'a, S>
where
    S: AsyncShared + ?Sized,
{
    pub value: Option<S::RefMut<'a>>,
    pub shared: &'a S,
}

unsafe impl<'a, S> Send for AsyncSharedGuardMut<'a, S> where S: AsyncShared + ?Sized {}

impl<'a, S> ops::Deref for AsyncSharedGuardMut<'a, S>
where
    S: AsyncShared,
{
    type Target = S::Value;

    fn deref(&self) -> &Self::Target {
        &self.value.as_ref().expect("unlocked")
    }
}

impl<'a, S> ops::DerefMut for AsyncSharedGuardMut<'a, S>
where
    S: AsyncShared,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value.as_deref_mut().expect("unlocked")
    }
}

impl<'a, S> AsyncSharedGuardMut<'a, S>
where
    S: AsyncShared + ?Sized,
{
    pub fn unlock(&mut self) {
        self.value.take();
    }

    pub async fn relock(&mut self) {
        self.value = self.shared.lock_mut_wait().await.value
    }
}

mod local;
pub use local::*;

mod mutex;
pub use mutex::*;
