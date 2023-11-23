use futures::Future;

use crate::io_device::IoDevice;

#[cfg(feature = "multi-thread")]
pub fn socket_tester<T, Fut>(test: T) -> Fut::Output
where
    T: FnOnce(futures::executor::ThreadPool, Option<IoDevice>) -> Fut,
    Fut: Future + Send + 'static,
    Fut::Output: Send,
{
    let thread_pool = futures::executor::ThreadPool::builder()
        .pool_size(40)
        .create()
        .unwrap();

    let future = test(thread_pool.clone(), None);

    use futures::task::SpawnExt;

    let handle = thread_pool.spawn_with_handle(future).unwrap();

    futures::executor::block_on(handle)
}

#[cfg(not(feature = "multi-thread"))]
pub fn socket_tester<T, Fut>(test: T) -> Fut::Output
where
    T: FnOnce(futures::executor::LocalSpawner, Option<IoDevice>) -> Fut,
    Fut: Future + 'static,
{
    use futures::{future::poll_fn, pin_mut};
    use std::task::Poll;

    let io_device = IoDevice::new().unwrap();

    let mut pool = futures::executor::LocalPool::new();

    let future = test(pool.spawner(), Some(io_device.clone()));

    pin_mut!(future);

    pool.run_until(poll_fn(|cx| loop {
        match future.as_mut().poll(cx) {
            Poll::Pending => {
                io_device.poll_once(None).unwrap();

                return Poll::Pending;
            }
            r => return r,
        }
    }))
}
