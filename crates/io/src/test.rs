pub use std::future::Future;
use std::sync::Once;

use crate::{current::register_driver, mio::mio_driver};

/// Test runner with multithread spawner and global poll event loop
pub fn io_test<T, Fut>(label: &'static str, test: T)
where
    T: FnOnce() -> Fut + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    static INIT: Once = Once::new();

    INIT.call_once(|| {
        register_driver(mio_driver()).unwrap();
    });

    log::trace!("start io test(st,{})", label);

    let fut = test();

    _ = super::current::executor::block_on(fut, 10);
}
