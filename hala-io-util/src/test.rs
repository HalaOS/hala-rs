use std::{cell::RefCell, io, rc::Rc, sync::Once};

use futures::{
    executor::{block_on, LocalPool, LocalSpawner, ThreadPool},
    future::{BoxFuture, LocalBoxFuture},
    task::LocalSpawnExt,
    Future,
};

use hala_io_driver::*;

static START: Once = Once::new();

use crate::*;

/// Test runner with multithread spawner and global poll event loop
pub fn io_test<T, Fut>(label: &'static str, test: T)
where
    T: FnOnce() -> Fut + 'static,
    Fut: Future<Output = ()> + 'static,
{
    START.call_once(|| {
        _ = register_driver(mio_driver());

        register_poller().unwrap();

        let thread_pool = ThreadPool::builder().pool_size(10).create().unwrap();

        register_spawner(TestIoSpawner(thread_pool)).unwrap();
    });

    let _guard = PollLoopGuard::new(None).unwrap();

    log::trace!("start io test(st,{})", label);

    block_on(test());
}

struct TestIoSpawner(ThreadPool);

impl IoSpawner for TestIoSpawner {
    type Fut = BoxFuture<'static, std::io::Result<()>>;
    fn spawn(&self, fut: Self::Fut) -> std::io::Result<()> {
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

/// Test runner with multithread spawner and global poll event loop
pub fn local_io_test<T, Fut>(label: &'static str, test: T)
where
    T: FnOnce() -> Fut + 'static,
    Fut: Future<Output = ()> + 'static,
{
    START.call_once(|| {
        _ = register_driver(mio_driver());

        register_poller().unwrap();

        let thread_pool = ThreadPool::builder().pool_size(10).create().unwrap();

        register_spawner(TestIoSpawner(thread_pool)).unwrap();
    });

    register_local_poller().unwrap();

    let mut pool = LocalPool::new();

    register_local_spawner(TestLocalIoSpawner(pool.spawner()));

    let dropping = Rc::new(RefCell::new(false));

    let dropping_cloned = dropping.clone();

    let start_fut = async move {
        test().await;

        *dropping_cloned.borrow_mut() = true;
    };

    pool.spawner()
        .spawn_local(start_fut)
        .map_err(|err| io::Error::new(io::ErrorKind::Other, format!("Spawn local error: {}", err)))
        .unwrap();

    let driver = get_driver().unwrap();
    let poller = get_local_poller().unwrap();

    log::trace!("start io test(st,{})", label);

    while !*dropping.borrow() {
        driver.fd_cntl(poller, Cmd::PollOnce(None)).unwrap();
        pool.run_until_stalled();
    }
}

struct TestLocalIoSpawner(LocalSpawner);

impl IoSpawner for TestLocalIoSpawner {
    type Fut = LocalBoxFuture<'static, std::io::Result<()>>;
    fn spawn(&self, fut: Self::Fut) -> std::io::Result<()> {
        self.0
            .spawn_local(async move {
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
