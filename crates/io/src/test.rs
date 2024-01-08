pub use std::future::Future;

/// Test runner with multithread spawner and global poll event loop
pub fn io_test<T, Fut>(label: &'static str, test: T)
where
    T: FnOnce() -> Fut + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    // _ = pretty_env_logger::try_init();

    log::trace!("start io test(st,{})", label);

    let fut = test();

    _ = crate::executor::block_on(fut, 10);
}
