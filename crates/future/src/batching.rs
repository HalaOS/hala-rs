use std::{
    future::Future,
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
use futures::{
    future::{poll_fn, BoxFuture},
    FutureExt,
};
use hala_lockfree::queue::Queue;

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
            let waker = unsafe { Box::from_raw(waker) };
            waker.wake();
        }
    }

    fn remove_waker(&self) -> Option<*mut Waker> {
        loop {
            let waker_ptr = self.waker.load(Ordering::Acquire);

            if waker_ptr == null_mut() {
                return None;
            }

            if self
                .waker
                .compare_exchange_weak(waker_ptr, null_mut(), Ordering::AcqRel, Ordering::Relaxed)
                .is_err()
            {
                continue;
            }

            return Some(waker_ptr);
        }
    }

    fn add_waker(&self, waker: Waker) {
        let waker_ptr = Box::into_raw(Box::new(waker));

        let old = self.waker.swap(waker_ptr, Ordering::AcqRel);

        if old != null_mut() {
            // TODO: check the data race!!!
            log::warn!("Batching is awakened unintentionally !!!.");
        }
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

fn new_batcher_waker<Fut>(future_id: usize, batch_future: FutureBatcher<Fut>) -> Waker {
    let boxed = Box::new(BatcherWaker {
        future_id,
        ready_futures: batch_future.wakeup_futures,
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
pub struct FutureBatcher<R> {
    /// The generator for the wrapped future id.
    idgen: Arc<AtomicUsize>,
    /// Current set of pending futures
    pending_futures: Arc<PendingFutures<R>>,
    /// Current set of ready futures
    wakeup_futures: Arc<Queue<usize>>,
    /// Raw batch future waker
    raw_waker: Arc<WakerHost>,

    await_counter: Arc<AtomicUsize>,
}

unsafe impl<R> Send for FutureBatcher<R> {}
unsafe impl<R> Sync for FutureBatcher<R> {}

impl<R> Clone for FutureBatcher<R> {
    fn clone(&self) -> Self {
        Self {
            idgen: self.idgen.clone(),
            pending_futures: self.pending_futures.clone(),
            wakeup_futures: self.wakeup_futures.clone(),
            raw_waker: self.raw_waker.clone(),
            await_counter: self.await_counter.clone(),
        }
    }
}

impl<R> Default for FutureBatcher<R> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R> FutureBatcher<R> {
    pub fn new() -> Self {
        Self {
            idgen: Default::default(),
            pending_futures: Default::default(),
            wakeup_futures: Default::default(),
            raw_waker: Default::default(),
            await_counter: Default::default(),
        }
    }

    /// Push a new task future.
    ///
    pub fn push<Fut>(&self, fut: Fut) -> usize
    where
        Fut: Future<Output = R> + Send + 'static,
    {
        let id = self.idgen.fetch_add(1, Ordering::AcqRel);

        self.pending_futures.insert(id, Box::pin(fut));
        self.wakeup_futures.push(id);

        self.raw_waker.wake();

        id
    }

    /// Use a fn_poll instead of a [`Future`] object
    pub fn push_fn<F>(&self, f: F) -> usize
    where
        F: FnMut(&mut Context<'_>) -> std::task::Poll<R> + Send + 'static,
    {
        self.push(poll_fn(f))
    }

    /// Create a future task to batch poll
    pub fn wait(&self) -> Wait<R> {
        Wait {
            batch: self.clone(),
        }
    }
}

pub struct Wait<R> {
    batch: FutureBatcher<R>,
}

impl<R> ops::Deref for Wait<R> {
    type Target = FutureBatcher<R>;
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
        assert_eq!(
            self.await_counter.fetch_add(1, Ordering::SeqCst),
            0,
            "Only one thread can call this batch poll"
        );

        // save system waker to prepare wakeup self again.
        self.raw_waker.add_waker(cx.waker().clone());

        while let Some(future_id) = self.wakeup_futures.pop() {
            // remove future from pending mapping.
            let future = self.pending_futures.remove(future_id);

            // The batcher waker may be register more than once.
            // thus it is possible that a future has been awakened more than once,
            // and that previous awakening processing has deleted that future.
            if future.is_none() {
                continue;
            }

            let mut future = future.unwrap();

            // Create a new wrapped waker.
            let waker = new_batcher_waker(future_id, self.clone());

            // poll if
            match future.poll_unpin(&mut Context::from_waker(&waker)) {
                std::task::Poll::Pending => {
                    self.pending_futures.insert(future_id, future);

                    continue;
                }
                std::task::Poll::Ready(r) => {
                    self.raw_waker.remove_waker();
                    assert_eq!(
                        self.await_counter.fetch_sub(1, Ordering::SeqCst),
                        1,
                        "Only one thread can call this batch poll"
                    );
                    return std::task::Poll::Ready(r);
                }
            }
        }

        assert_eq!(
            self.await_counter.fetch_sub(1, Ordering::SeqCst),
            1,
            "Only one thread can call this batch poll"
        );

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
        let batch_future = FutureBatcher::<io::Result<()>>::new();

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

        let batch_future = FutureBatcher::<io::Result<()>>::new();

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

        let batch_future = FutureBatcher::<io::Result<()>>::new();

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
