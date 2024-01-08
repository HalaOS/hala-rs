use std::{
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Once, OnceLock,
    },
};

use futures::{executor::ThreadPool, task::SpawnExt};
use hala_future::BoxFuture;

use std::future::Future;

use crate::{current::*, Cmd};

pub static INIT_DRIVER: Once = Once::new();

static POOL: OnceLock<ThreadPool> = OnceLock::new();

/// Run a hala io future for multi-thread mode.
pub fn block_on<Fut, R>(fut: Fut, pool_size: usize) -> R
where
    Fut: Future<Output = R> + Send + 'static,
    R: Send + 'static,
{
    use crate::mio::mio_driver;

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

pub struct BlockOnIoSpawner(pub ThreadPool);

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
