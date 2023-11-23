use futures::Future;

static POOL: std::sync::OnceLock<futures::executor::ThreadPool> = std::sync::OnceLock::new();

pub fn socket_tester<T, Fut>(test: T)
where
    T: FnOnce() -> Fut,
    Fut: Future + Send + 'static,
    Fut::Output: Send,
{
    let thread_pool = spawner();

    let future = test();

    use futures::task::SpawnExt;

    let handle = thread_pool.spawn_with_handle(future).unwrap();

    futures::executor::block_on(handle);
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
