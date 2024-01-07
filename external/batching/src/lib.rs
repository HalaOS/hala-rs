use std::{
    ops,
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
struct PendingFutures<R> {
    futures: DashMap<usize, BoxFuture<'static, R>>,
}

impl<R> PendingFutures<R> {
    fn insert(&self, id: usize, fut: BoxFuture<'static, R>) {
        self.futures.insert(id, fut);
    }

    fn remove(&self, id: usize) -> Option<BoxFuture<'static, R>> {
        self.futures.remove(&id).map(|(_, fut)| fut)
    }
}

impl<R> Default for PendingFutures<R> {
    fn default() -> Self {
        Self {
            futures: DashMap::new(),
        }
    }
}

#[derive(Default)]
struct WakerHost {
    waker: AtomicPtr<Waker>,
}

impl WakerHost {
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
struct BatcherWaker {
    future_id: usize,
    /// Current set of ready futures
    ready_futures: Arc<Queue<usize>>,
    /// Raw batch future waker
    raw_waker: Arc<WakerHost>,
}

#[inline(always)]
unsafe fn batch_future_waker_clone(data: *const ()) -> RawWaker {
    let waker = Box::from_raw(data as *mut BatcherWaker);

    let waker_cloned = waker.clone();

    _ = Box::into_raw(waker);

    RawWaker::new(Box::into_raw(waker_cloned) as *const (), &WAKER_VTABLE)
}

#[inline(always)]
unsafe fn batch_future_waker_wake(data: *const ()) {
    let waker = Box::from_raw(data as *mut BatcherWaker);

    waker.ready_futures.push(waker.future_id);

    waker.raw_waker.wake();
}

#[inline(always)]
unsafe fn batch_future_waker_wake_by_ref(data: *const ()) {
    let waker = Box::from_raw(data as *mut BatcherWaker);

    waker.ready_futures.push(waker.future_id);

    waker.raw_waker.wake();

    _ = Box::into_raw(waker);
}

#[inline(always)]
unsafe fn batch_future_waker_drop(data: *const ()) {
    _ = Box::from_raw(data as *mut BatcherWaker);
}

const WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(
    batch_future_waker_clone,
    batch_future_waker_wake,
    batch_future_waker_wake_by_ref,
    batch_future_waker_drop,
);

fn new_batcher_waker<Fut>(future_id: usize, batch_future: Batcher<Fut>) -> Waker {
    let boxed = Box::new(BatcherWaker {
        future_id,
        ready_futures: batch_future.ready_futures,
        raw_waker: batch_future.raw_waker,
    });

    unsafe {
        Waker::from_raw(RawWaker::new(
            Box::into_raw(boxed) as *const (),
            &WAKER_VTABLE,
        ))
    }
}

/// A lockfree processor to batch poll the same type of futures.
pub struct Batcher<R> {
    /// The generator for the wrapped future id.
    idgen: Arc<AtomicUsize>,
    /// Current set of pending futures
    pending_futures: Arc<PendingFutures<R>>,
    /// Current set of ready futures
    ready_futures: Arc<Queue<usize>>,
    /// Raw batch future waker
    raw_waker: Arc<WakerHost>,
}

unsafe impl<R> Send for Batcher<R> {}
unsafe impl<R> Sync for Batcher<R> {}

impl<R> Clone for Batcher<R> {
    fn clone(&self) -> Self {
        Self {
            idgen: self.idgen.clone(),
            pending_futures: self.pending_futures.clone(),
            ready_futures: self.ready_futures.clone(),
            raw_waker: self.raw_waker.clone(),
        }
    }
}

impl<R> Default for Batcher<R> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R> Batcher<R> {
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
        Fut: Future<Output = R> + Send + 'static,
    {
        let id = self.idgen.fetch_add(1, Ordering::AcqRel);

        self.pending_futures.insert(id, Box::pin(fut));
        self.ready_futures.push(id);

        self.raw_waker.wake();

        id
    }

    /// Create a future task to batch poll
    pub fn wait(&self) -> Wait<R> {
        Wait {
            batch: self.clone(),
        }
    }
}

pub struct Wait<R> {
    batch: Batcher<R>,
}

impl<R> ops::Deref for Wait<R> {
    type Target = Batcher<R>;
    fn deref(&self) -> &Self::Target {
        &self.batch
    }
}

impl<R> Future for Wait<R> {
    type Output = R;
    fn poll(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        // save system waker to prepare wakeup self again.
        self.raw_waker.add_waker(cx.waker().clone());

        while let Some(ready) = self.ready_futures.pop() {
            // remove future from pending mapping.
            let mut future = self
                .pending_futures
                .remove(ready)
                .expect("Calling BatchFuture.await in multi-threads is not allowed");

            // Create a new wrapped waker.
            let waker = new_batcher_waker(ready, self.clone());

            // poll if
            match future.poll_unpin(&mut Context::from_waker(&waker)) {
                std::task::Poll::Pending => {
                    self.pending_futures.insert(ready, future);

                    continue;
                }
                std::task::Poll::Ready(r) => {
                    self.raw_waker.remove_waker();
                    return std::task::Poll::Ready(r);
                }
            }
        }

        return std::task::Poll::Pending;
    }
}

#[cfg(test)]
mod tests {

    use std::{io, sync::mpsc};

    use futures::{executor::ThreadPool, future::poll_fn, task::SpawnExt};

    use super::*;

    #[futures_test::test]
    async fn test_basic_case() {
        let batch_future = Batcher::<io::Result<()>>::new();

        let loops = 100000;

        for _ in 0..loops {
            batch_future.push(async { Ok(()) });
            batch_future.push(async move { Ok(()) });

            batch_future.wait().await.unwrap();

            batch_future.wait().await.unwrap();
        }
    }

    #[futures_test::test]
    async fn test_push_wakeup() {
        let pool = ThreadPool::builder().pool_size(10).create().unwrap();

        let batch_future = Batcher::<io::Result<()>>::new();

        let loops = 100000;

        for _ in 0..loops {
            let batch_future_cloned = batch_future.clone();

            let handle = pool
                .spawn_with_handle(async move {
                    batch_future_cloned.wait().await.unwrap();
                })
                .unwrap();

            batch_future.push(async move { Ok(()) });

            handle.await;
        }
    }

    #[futures_test::test]
    async fn test_future_wakeup() {
        let pool = ThreadPool::builder().pool_size(10).create().unwrap();

        let batch_future = Batcher::<io::Result<()>>::new();

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
                    batch_futre_cloned.wait().await.unwrap();
                })
                .unwrap();

            let waker = receiver.recv().unwrap();

            waker.wake();

            handle.await;
        }
    }
}
