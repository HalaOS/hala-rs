pub use std::future::Future;
use std::{io, time::Instant};

use hala_future::executor::block_on;

/// Test runner with multithread spawner and global poll event loop
pub fn io_test<T, Fut>(label: &'static str, test: T)
where
    T: FnOnce() -> Fut + 'static,
    Fut: Future<Output = io::Result<()>> + Send + 'static,
{
    let fut = test();

    println!("io_test {}", label);

    let start = Instant::now();

    match block_on(fut, 10) {
        Ok(_) => {
            println!("io_test {} ... finished in {:?}", label, start.elapsed());
        }
        Err(err) => {
            println!("io_test {} catch error:", label);
            println!("{}", err);
        }
    }
}
