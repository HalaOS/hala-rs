pub use std::future::Future;

use hala_future::executor::block_on;

/// Test runner with multithread spawner and global poll event loop
pub fn io_test<T, Fut>(_: &'static str, test: T)
where
    T: FnOnce() -> Fut + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let fut = test();

    block_on(fut, 10);
}
