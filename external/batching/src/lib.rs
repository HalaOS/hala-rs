use std::{
    io, ops,
    pin::Pin,
    ptr::null_mut,
    sync::{
        atomic::{AtomicPtr, AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, RawWaker, RawWakerVTable, Waker},
};

use dashmap::DashMap;
use futures::{future::BoxFuture, Future, FutureExt};
use lockfree_rs::queue::Queue;

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
struct RawBatchFutureWaker {
    waker: AtomicPtr<Waker>,
}

impl RawBatchFutureWaker {
    fn wake(&self) {
        if let Some(waker) = self.remove_waker() {
            waker.wake();
        }
    }

    fn remove_waker(&self) -> Option<Box<Waker>> {
        let waker_ptr = self.waker.load(Ordering::Acquire);

        if waker_ptr == null_mut() {
            return None;
        }

        if self
            .waker
            .compare_exchange(waker_ptr, null_mut(), Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            return Some(unsafe { Box::from_raw(waker_ptr) });
        }

        return None;
    }

    fn add_waker(&self, waker: Waker) {
        let waker_ptr = Box::into_raw(Box::new(waker));

        self.waker
            .compare_exchange(null_mut(), waker_ptr, Ordering::AcqRel, Ordering::Relaxed)
            .expect("Add waker ordering check failed");
    }
}

#[derive(Clone)]
struct BatchFutureWaker {
    future_id: usize,
    batch_future: Batcher,
}

impl BatchFutureWaker {
    fn new(future_id: usize, batch_future: Batcher) -> Self {
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

    waker.batch_future.raw_waker.wake();

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

fn new_batch_futre_waker(future_id: usize, batch_future: Batcher) -> Waker {
    let boxed = Box::new(BatchFutureWaker::new(future_id, batch_future));

    unsafe {
        Waker::from_raw(RawWaker::new(
            Box::into_raw(boxed) as *const (),
            &WAKER_VTABLE,
        ))
    }
}

/// A lockfree processor to batch poll the same type of futures.
pub struct Batcher {
    /// The generator for the wrapped future id.
    idgen: Arc<AtomicUsize>,
    /// Current set of pending futures
    pending_futures: Arc<PendingFutures>,
    /// Current set of ready futures
    ready_futures: Arc<Queue<usize>>,
    /// Raw batch future waker
    raw_waker: Arc<RawBatchFutureWaker>,
}

unsafe impl Send for Batcher {}
unsafe impl Sync for Batcher {}

impl Clone for Batcher {
    fn clone(&self) -> Self {
        Self {
            idgen: self.idgen.clone(),
            pending_futures: self.pending_futures.clone(),
            ready_futures: self.ready_futures.clone(),
            raw_waker: self.raw_waker.clone(),
        }
    }
}

impl Default for Batcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Batcher {
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

    /// Create a future task to batch poll
    pub fn wait(&self) -> Wait {
        Wait {
            batch: self.clone(),
        }
    }
}

pub struct Wait {
    batch: Batcher,
}

impl ops::Deref for Wait {
    type Target = Batcher;
    fn deref(&self) -> &Self::Target {
        &self.batch
    }
}

impl Future for Wait {
    type Output = (usize, io::Result<()>);
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
                std::task::Poll::Ready(r) => {
                    self.raw_waker.remove_waker();

                    return std::task::Poll::Ready((ready, r));
                }
            }
        }

        return std::task::Poll::Pending;
    }
}

#[cfg(test)]
mod tests {

    use std::sync::mpsc;

    use futures::{executor::ThreadPool, future::poll_fn, task::SpawnExt};

    use super::*;

    #[futures_test::test]
    async fn test_basic_case() {
        let batch_future = Batcher::new();

        let loops = 100000;

        for _ in 0..loops {
            batch_future.push(async { Ok(()) });
            batch_future.push(async move { Ok(()) });

            batch_future.wait().await.1.unwrap();

            batch_future.wait().await.1.unwrap();
        }
    }

    #[futures_test::test]
    async fn test_push_wakeup() {
        let pool = ThreadPool::builder().pool_size(10).create().unwrap();

        let batch_future = Batcher::new();

        let loops = 100000;

        for _ in 0..loops {
            let batch_future_cloned = batch_future.clone();

            let handle = pool
                .spawn_with_handle(async move {
                    batch_future_cloned.wait().await.1.unwrap();
                })
                .unwrap();

            batch_future.push(async move { Ok(()) });

            handle.await;
        }
    }

    #[futures_test::test]
    async fn test_future_wakeup() {
        let pool = ThreadPool::builder().pool_size(10).create().unwrap();

        let batch_future = Batcher::new();

        for _ in 0..10000 {
            let (sender, receiver) = mpsc::channel();

            let mut sent = false;

            batch_future.push(poll_fn(move |cx| {
                if sent {
                    return std::task::Poll::Ready(Ok(()));
                }

                sender.send(cx.waker().clone()).unwrap();

                sent = true;

                std::task::Poll::Pending
            }));

            let batch_futre_cloned = batch_future.clone();

            let handle = pool
                .spawn_with_handle(async move {
                    batch_futre_cloned.wait().await.1.unwrap();
                })
                .unwrap();

            let waker = receiver.recv().unwrap();

            waker.wake();

            handle.await;
        }
    }
}
