use std::future::Future;

use hala_future::BoxFuture;

use super::*;

use std::{io, sync::OnceLock};

static DRIVER: OnceLock<Driver> = OnceLock::new();

/// Get the currently registered io driver, or return a NotFound error if it is not registered.
pub fn get_driver() -> io::Result<Driver> {
    return DRIVER
        .get()
        .map(|driver| driver.clone())
        .ok_or(io::Error::new(
            io::ErrorKind::NotFound,
            "[Hala-IO] call register_local_driver/register_driver first",
        ));
}

/// Register new io driver
pub fn register_driver<D: Into<Driver>>(driver: D) -> io::Result<()> {
    DRIVER.set(driver.into()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "[Hala-IO] call register_driver twice",
        )
    })
}

static POLLER: OnceLock<Poller> = OnceLock::new();

struct Poller(Handle);

impl Drop for Poller {
    fn drop(&mut self) {
        get_driver().unwrap().fd_close(self.0).unwrap();
    }
}

/// Get poller instance from global context.
pub fn get_poller() -> io::Result<Handle> {
    let poller = POLLER.get_or_init(|| {
        let driver = get_driver().expect("call register_driver first");

        let handle = driver
            .fd_open(Description::Poller, OpenFlags::None)
            .unwrap();

        Poller(handle)
    });

    Ok(poller.0)
}

/// The `IoSpawner` trait allows for pushing an io futures onto an executor that will
/// run them to completion.
pub trait IoSpawner {
    type Fut: Future<Output = io::Result<()>>;
    /// Spawns a io task that polls the given future with output `io::Result<()>` to completion.
    fn spawn(&self, fut: Self::Fut) -> io::Result<()>;
}

static SPAWNER: OnceLock<
    Box<dyn IoSpawner<Fut = BoxFuture<'static, io::Result<()>>> + Send + Sync>,
> = OnceLock::new();

/// Register [`IoSpawner`] for all thread. the `IoSpawner` instance must implement [`Send`] + [`Sync`] traits.
pub fn register_spawner<
    S: IoSpawner<Fut = BoxFuture<'static, io::Result<()>>> + Send + Sync + 'static,
>(
    spawner: S,
) -> io::Result<()> {
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
