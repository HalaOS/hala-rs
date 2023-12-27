use std::future::Future;

use crate::Shared;

/// `Shared` extension trait for asynchronization program.
pub trait AsyncShared: Shared {
    /// Future type created by [`lock_wait`](AsyncShared::lock_wait)
    type RefFuture<'a>: Future<Output = Self::Ref<'a>>
    where
        Self: 'a;

    /// Future type created by [`lock_mut_wait`](AsyncShared::lock_wait)
    type RefMutFuture<'a>: Future<Output = Self::RefMut<'a>>
    where
        Self: 'a;

    /// Create new future and wait locking shared value and getting immutable reference.
    fn lock_wait(&self) -> Self::RefFuture<'_>;

    /// Create new future and wait locking shared value and getting mutable reference.
    fn lock_mut_wait(&self) -> Self::RefMutFuture<'_>;
}

mod local;
pub use local::*;

mod mutex;
pub use mutex::*;
