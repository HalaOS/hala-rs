use std::{
    io,
    pin::Pin,
    sync::{
        atomic::{AtomicPtr, AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, RawWaker, RawWakerVTable, Waker},
};

use dashmap::DashMap;
use futures::{future::BoxFuture, Future, FutureExt};

/// A set to handle the reigstration/deregistration of pending future.
struct PendingFutures {
    futures: DashMap<usize, BoxFuture<'static, io::Result<()>>>,
}

impl PendingFutures {
    fn insert(&self, id: usize, fut: BoxFuture<'static, io::Result<()>>) {
        self.futures.insert(id, fut);
    }

    fn remove(&self, id: usize) -> Option<BoxFuture<'static, io::Result<()>>> {
        self.futures.remove(&id).map(|(_, fut)| fut)
    }
}

impl Default for PendingFutures {
    fn default() -> Self {
        Self {
            futures: DashMap::new(),
        }
    }
}

#[derive(Default)]
struct ReadyFutures {
    futures: DashMap<usize, usize>,
}

impl ReadyFutures {
    fn push(&self, future_id: usize) {}

    fn pop(&self) -> Option<usize> {
        None
    }
}

#[derive(Default)]
struct RawBatchFutureWaker {}

impl RawBatchFutureWaker {
    fn wake(&self) {}

    fn wake_by_ref(&self) {}

    fn add_waker(&self, waker: Waker) {}
}

#[derive(Clone)]
struct BatchFutureWaker {
    future_id: usize,
    batch_future: BatchFuture,
}

impl BatchFutureWaker {
    fn new(future_id: usize, batch_future: BatchFuture) -> Self {
        Self {
            future_id,
            batch_future,
        }
    }
}

unsafe fn batch_future_waker_clone(data: *const ()) -> RawWaker {
    let waker = Box::from_raw(data as *mut BatchFutureWaker);

    let waker_cloned = waker.clone();

    _ = Box::into_raw(waker);

    RawWaker::new(Box::into_raw(waker_cloned) as *const (), &WAKER_VTABLE)
}

unsafe fn batch_future_waker_wake(data: *const ()) {
    let waker = Box::from_raw(data as *mut BatchFutureWaker);

    waker.batch_future.ready_futures.push(waker.future_id);

    waker.batch_future.raw_waker.wake();
}

unsafe fn batch_future_waker_wake_by_ref(data: *const ()) {
    let waker = Box::from_raw(data as *mut BatchFutureWaker);

    waker.batch_future.ready_futures.push(waker.future_id);

    waker.batch_future.raw_waker.wake_by_ref();

    _ = Box::into_raw(waker);
}

unsafe fn batch_future_waker_drop(data: *const ()) {
    _ = Box::from_raw(data as *mut BatchFutureWaker);
}

const WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(
    batch_future_waker_clone,
    batch_future_waker_wake,
    batch_future_waker_wake_by_ref,
    batch_future_waker_drop,
);

fn new_batch_futre_waker(future_id: usize, batch_future: BatchFuture) -> Waker {
    let boxed = Box::new(BatchFutureWaker::new(future_id, batch_future));

    unsafe {
        Waker::from_raw(RawWaker::new(
            Box::into_raw(boxed) as *const (),
            &WAKER_VTABLE,
        ))
    }
}

/// A lockfree processor to batch poll the same type of futures.
pub struct BatchFuture {
    /// The generator for the wrapped future id.
    idgen: Arc<AtomicUsize>,
    /// Current set of pending futures
    pending_futures: Arc<PendingFutures>,
    /// Current set of ready futures
    ready_futures: Arc<ReadyFutures>,
    /// Raw batch future waker
    raw_waker: Arc<RawBatchFutureWaker>,
}

impl Clone for BatchFuture {
    fn clone(&self) -> Self {
        Self {
            idgen: self.idgen.clone(),
            pending_futures: self.pending_futures.clone(),
            ready_futures: self.ready_futures.clone(),
            raw_waker: self.raw_waker.clone(),
        }
    }
}

impl BatchFuture {
    pub fn new() -> Self {
        Self {
            idgen: Default::default(),
            pending_futures: Default::default(),
            ready_futures: Default::default(),
            raw_waker: Default::default(),
        }
    }
    pub fn push<Fut>(&self, fut: Fut) -> usize
    where
        Fut: Future<Output = io::Result<()>> + Send + 'static,
    {
        let id = self.idgen.fetch_add(1, Ordering::AcqRel);

        self.pending_futures.insert(id, Box::pin(fut));
        self.ready_futures.push(id);

        self.raw_waker.wake();

        id
    }
}

impl Future for BatchFuture {
    type Output = io::Result<()>;
    fn poll(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        self.raw_waker.add_waker(cx.waker().clone());

        while let Some(ready) = self.ready_futures.pop() {
            let mut future = self
                .pending_futures
                .remove(ready)
                .expect("Calling BatchFuture.await in multi-threads is not allowed");

            let waker = new_batch_futre_waker(ready, self.clone());

            match future.poll_unpin(&mut Context::from_waker(&waker)) {
                std::task::Poll::Pending => {
                    self.pending_futures.insert(ready, future);
                    continue;
                }
                poll => return poll,
            }
        }

        return std::task::Poll::Pending;
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_default() {
        let batch_future = BatchFuture::new();

        batch_future.push(async { Ok(()) });
        batch_future.push(async move { Ok(()) });
    }
}
