use std::task::Poll;

use futures::{future::BoxFuture, Future};

static POOL: std::sync::OnceLock<futures::executor::ThreadPool> = std::sync::OnceLock::new();

pub fn socket_tester<T, Fut>(test: T)
where
    T: FnOnce() -> Fut,
    Fut: Future + Send + 'static,
    Fut::Output: Send,
{
    _ = pretty_env_logger::try_init();

    let thread_pool = spawner();

    use futures::task::SpawnExt;

    let handle = if IoDevice::is_multithread() {
        log::trace!("start multi-thread io test");

        global_io_device().start(None);

        thread_pool.spawn_with_handle(test()).unwrap()
    } else {
        log::trace!("start single thread io test");

        let future = test();

        let future = SingleThreadFutureWrapper {
            future: Box::pin(future),
        };

        thread_pool.spawn_with_handle(future).unwrap()
    };

    futures::executor::block_on(handle);
}

struct SingleThreadFutureWrapper<Output> {
    future: BoxFuture<'static, Output>,
}

impl<Output> Future for SingleThreadFutureWrapper<Output>
where
    Output: Send,
{
    type Output = Output;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Self::Output> {
        match self.future.as_mut().poll(cx) {
            Poll::Pending => {
                global_io_device().poll_once(None).unwrap();
                return Poll::Pending;
            }
            r => return r,
        }
    }
}

pub fn spawner() -> &'static futures::executor::ThreadPool {
    let pool_size = if cfg!(feature = "multi-thread") {
        10
    } else {
        1
    };

    POOL.get_or_init(|| {
        futures::executor::ThreadPool::builder()
            .pool_size(pool_size)
            .create()
            .unwrap()
    })
}

pub use futures;

pub use hala_io_test_derive::*;
use hala_reactor::{global_io_device, IoDevice};
