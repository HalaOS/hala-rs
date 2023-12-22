mod driver;
use std::{
    cell::RefCell,
    io,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Once, OnceLock,
    },
};

pub use driver::*;

mod poller;
use futures::{
    executor::{LocalPool, LocalSpawner, ThreadPool},
    future::{BoxFuture, LocalBoxFuture},
    task::{LocalSpawnExt, SpawnExt},
    Future,
};
use hala_io_driver::{mio_driver, Cmd};
pub use poller::*;

mod spawner;
pub use spawner::*;

static INIT_DRIVER: Once = Once::new();
static POOL: OnceLock<ThreadPool> = OnceLock::new();

/// Run a hala io future for multi-thread mode.
pub fn block_on<Fut, R>(fut: Fut, pool_size: usize) -> R
where
    Fut: Future<Output = R> + Send + 'static,
    R: Send + 'static,
{
    INIT_DRIVER.call_once(|| {
        register_driver(mio_driver()).unwrap();
    });

    let pool = POOL.get_or_init(|| {
        let pool = ThreadPool::builder()
            .pool_size(pool_size)
            .create()
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))
            .unwrap();

        register_spawner(BlockOnIoSpawner(pool.clone())).unwrap();

        pool
    });

    let dropping = Arc::new(AtomicBool::new(false));

    let dropping_cloned = dropping.clone();

    let handle = pool
        .spawn_with_handle(fut)
        .map_err(|err| io::Error::new(io::ErrorKind::Other, format!("Spawn local error: {}", err)))
        .unwrap();

    let driver = get_driver().unwrap();
    let poller = get_poller().unwrap();

    std::thread::spawn(move || {
        while !dropping_cloned.load(Ordering::SeqCst) {
            driver.fd_cntl(poller, Cmd::PollOnce(None)).unwrap();
        }
    });

    futures::executor::block_on(handle)
}

struct BlockOnIoSpawner(ThreadPool);

impl IoSpawner for BlockOnIoSpawner {
    type Fut = BoxFuture<'static, std::io::Result<()>>;
    fn spawn(&self, fut: Self::Fut) -> std::io::Result<()> {
        self.0
            .spawn(async move {
                match fut.await {
                    Ok(_) => {}
                    Err(err) => {
                        log::error!("{}", err);
                    }
                }
            })
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))
    }
}

/// Run a hala io future for single-thread mode.
pub fn local_block_on<Fut, R>(fut: Fut) -> R
where
    Fut: Future<Output = R> + 'static,
    R: 'static,
{
    INIT_DRIVER.call_once(|| {
        register_driver(mio_driver()).unwrap();
    });

    let mut pool = LocalPool::new();

    register_local_spawner(LocalBlockOnIoSpawner(pool.spawner()));

    let dropping = Rc::new(RefCell::new(false));

    let dropping_cloned = dropping.clone();

    let start_fut = async move {
        let r = fut.await;

        *dropping_cloned.borrow_mut() = true;

        r
    };

    let handle = pool
        .spawner()
        .spawn_local_with_handle(start_fut)
        .map_err(|err| io::Error::new(io::ErrorKind::Other, format!("Spawn local error: {}", err)))
        .unwrap();

    let driver = get_driver().unwrap();
    let poller = get_local_poller().unwrap();

    while !*dropping.borrow() {
        driver.fd_cntl(poller, Cmd::PollOnce(None)).unwrap();
        pool.run_until_stalled();
    }

    pool.run_until(handle)
}

struct LocalBlockOnIoSpawner(LocalSpawner);

impl IoSpawner for LocalBlockOnIoSpawner {
    type Fut = LocalBoxFuture<'static, std::io::Result<()>>;
    fn spawn(&self, fut: LocalBoxFuture<'static, std::io::Result<()>>) -> std::io::Result<()> {
        self.0
            .spawn_local(async move {
                match fut.await {
                    Ok(_) => {}
                    Err(err) => {
                        log::error!("{}", err);
                    }
                }
            })
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_futures_executor() {
        let a = async {
            log::trace!("hello world");
        };

        let b = async move {
            a.await;

            Ok::<(), ()>(())
        };

        let c = async move {
            b.await.unwrap();
        };

        futures::executor::block_on(c);
    }
}
