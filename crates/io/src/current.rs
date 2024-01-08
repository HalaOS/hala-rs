use std::future::Future;

use futures::future::BoxFuture;

use super::*;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use std::{io, sync::OnceLock};

static DRIVER: OnceLock<Driver> = OnceLock::new();

/// Get the global context registered io driver, or return a NotFound error if it is not registered.
pub fn get_driver() -> io::Result<Driver> {
    return DRIVER
        .get()
        .map(|driver| driver.clone())
        .ok_or(io::Error::new(
            io::ErrorKind::NotFound,
            "[Hala-IO] call register_local_driver/register_driver first",
        ));
}

/// Register a new [`Driver`] implementation to global context.
pub fn register_driver<D: Into<Driver>>(driver: D) -> io::Result<()> {
    DRIVER.set(driver.into()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "[Hala-IO] call register_driver twice",
        )
    })
}

struct Poller(Handle);

impl Drop for Poller {
    fn drop(&mut self) {
        get_driver().unwrap().fd_close(self.0).unwrap();
    }
}

/// Get poller [`Handle`] from global context.
///
/// Based on lazy optimizations, the Poller instance is not created until the first call to the function.
pub fn get_poller() -> io::Result<Handle> {
    static POLLER: OnceLock<Poller> = OnceLock::new();

    let poller = POLLER.get_or_init(|| {
        let driver = get_driver().expect("call register_driver first");

        let handle = driver
            .fd_open(Description::Poller, OpenFlags::None)
            .unwrap();

        Poller(handle)
    });

    Ok(poller.0)
}

pub mod executor {

    use super::*;

    use futures::{executor::ThreadPool, task::SpawnExt};

    /// The `IoSpawner` trait allows for pushing an io futures onto an executor that will
    /// run them to completion.
    pub trait IoSpawner {
        /// Spawns a io task that polls the given future with output `io::Result<()>` to completion.
        fn spawn(&self, fut: BoxFuture<'static, io::Result<()>>) -> io::Result<()>;
    }

    static SPAWNER: OnceLock<Box<dyn IoSpawner + Send + Sync + 'static>> = OnceLock::new();

    /// Register [`IoSpawner`] for all thread. the `IoSpawner` instance must implement [`Send`] + [`Sync`] traits.
    pub fn register_spawner<S: IoSpawner + Send + Sync + 'static>(spawner: S) -> io::Result<()> {
        if let Err(_) = SPAWNER.set(Box::new(spawner)) {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "Call register_spawner more than once",
            ));
        }

        Ok(())
    }

    /// Spawn an io task that polls the given future with output `io::Result<()>` to completion.
    pub fn io_spawn<Fut>(fut: Fut) -> io::Result<()>
    where
        Fut: Future<Output = io::Result<()>> + Send + 'static,
    {
        if let Some(spawner) = SPAWNER.get() {
            return spawner.spawn(Box::pin(fut));
        }

        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Call register_local_spawner / register_spawner first",
        ));
    }

    /// Start a io future task and block current thread until this future ready.
    pub fn block_on<Fut, R>(fut: Fut, pool_size: usize) -> R
    where
        Fut: Future<Output = R> + Send + 'static,
        R: Send + 'static,
    {
        static POOL: OnceLock<ThreadPool> = OnceLock::new();

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
            .map_err(|err| {
                io::Error::new(io::ErrorKind::Other, format!("Spawn local error: {}", err))
            })
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
        fn spawn(&self, fut: BoxFuture<'static, io::Result<()>>) -> std::io::Result<()> {
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
}
