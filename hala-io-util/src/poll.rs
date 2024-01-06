use std::{
    future::IntoFuture,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize},
        Arc,
    },
};

use dashmap::DashMap;
use futures::FutureExt;

/// A future will polling a wrapped future once within the current async context.
pub struct PollOnce<Fut> {
    fut: Pin<Box<Fut>>,
}

impl<T> From<T> for PollOnce<T::IntoFuture>
where
    T: IntoFuture,
{
    fn from(value: T) -> Self {
        Self::new(value.into_future())
    }
}

impl<Fut> PollOnce<Fut> {
    pub fn new(fut: Fut) -> Self {
        Self { fut: Box::pin(fut) }
    }
}

impl<Fut, R> std::future::Future for PollOnce<Fut>
where
    Fut: std::future::Future<Output = R>,
{
    type Output = std::task::Poll<R>;
    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::task::Poll::Ready(self.fut.poll_unpin(cx))
    }
}

#[macro_export]
macro_rules! poll_once {
    ($fut: expr) => {
        $crate::PollOnce::new($fut).await
    };
}

struct BatchFutureWrapper<Fut> {
    /// the orignal future instance of this wrapper.
    orignal_future: Pin<Box<Fut>>,
    /// the wake-up call flag
    flag: Arc<AtomicBool>,
}

/// A future batch poll same type futures.
pub struct BatchFuture<Fut> {
    /// The generator for the wrapped future id.
    idgen: Arc<AtomicUsize>,
    /// Current set of pending futures
    pending_futures: DashMap<usize, BatchFutureWrapper<Fut>>,
    /// Current set of ready futures
    wakeup_futures: boxcar::Vec<BatchFutureWrapper<Fut>>,
}
