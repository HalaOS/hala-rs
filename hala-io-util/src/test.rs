use std::{io, sync::Once};

use futures::{
    executor::{block_on, ThreadPool},
    future::BoxFuture,
    Future,
};

use hala_io_driver::*;

static START: Once = Once::new();

use crate::*;

/// Test runner with local spawner and local poller event loop
pub fn io_test<T, Fut>(label: &'static str, test: T)
where
    T: FnOnce() -> Fut + 'static,
    Fut: Future<Output = ()> + 'static,
{
    START.call_once(|| {
        _ = register_driver(mio_driver());

        let thread_pool = ThreadPool::builder().pool_size(10).create().unwrap();

        register_spawner(TestIoSpawner(thread_pool));
    });

    let _guard = PollerLoopGuard::new(None).unwrap();

    log::trace!("start io test(st,{})", label);

    block_on(test());
}

struct TestIoSpawner(ThreadPool);

impl IoSpawner for TestIoSpawner {
    fn spawn(&self, fut: BoxFuture<'static, std::io::Result<()>>) -> std::io::Result<()> {
        use futures::task::SpawnExt;

        self.0
            .spawn(async move {
                match fut.await {
                    Ok(_) => {}
                    Err(err) => {
                        log::error!("IoSpawner catch err={}", err);
                    }
                }
            })
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))
    }
}
