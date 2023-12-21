use std::cell::RefCell;

use futures::{
    future::{BoxFuture, LocalBoxFuture},
    Future,
};

use std::{io, sync::OnceLock};

/// The `IoSpawner` trait allows for pushing an io futures onto an executor that will
/// run them to completion.
pub trait IoSpawner {
    type Fut: Future<Output = io::Result<()>>;
    /// Spawns a io task that polls the given future with output `io::Result<()>` to completion.
    fn spawn(&self, fut: Self::Fut) -> io::Result<()>;
}

thread_local! {
   pub(super) static LOCAL_SPAWNER: RefCell<Option<Box<dyn IoSpawner<Fut = LocalBoxFuture<'static,io::Result<()>>>>>> = RefCell::new(None);
}

static SPAWNER: OnceLock<
    Box<dyn IoSpawner<Fut = BoxFuture<'static, io::Result<()>>> + Send + Sync>,
> = OnceLock::new();

/// Register [`IoSpawner`] for local thread.
pub fn register_local_spawner<
    S: IoSpawner<Fut = LocalBoxFuture<'static, io::Result<()>>> + 'static,
>(
    spawner: S,
) {
    LOCAL_SPAWNER.set(Some(Box::new(spawner)));
}

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
    LOCAL_SPAWNER.with_borrow(|spawner| {
        if let Some(spawner) = spawner {
            return spawner.spawn(Box::pin(fut));
        }

        if let Some(spawner) = SPAWNER.get() {
            return spawner.spawn(Box::pin(fut));
        }

        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Call register_local_spawner / register_spawner first",
        ));
    })
}

/// Spawn an io task onto local spawner.
pub fn local_io_spawn<Fut>(fut: Fut) -> io::Result<()>
where
    Fut: Future<Output = io::Result<()>> + 'static,
{
    LOCAL_SPAWNER.with_borrow(|spawner| {
        if let Some(spawner) = spawner {
            return spawner.spawn(Box::pin(fut));
        }

        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Call register_local_spawner  first",
        ));
    })
}
