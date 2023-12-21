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
/// ```
/// # {
/// use hala_io_util::*;
/// use std::io;
/// use futures::{
///     executor::{LocalPool, LocalSpawner},
///     future::BoxFuture,
/// };
///
/// struct MockIoSpawner {
///     spawner: LocalSpawner,
/// }
///
/// impl IoSpawner for MockIoSpawner {
///     fn spawn(&self, fut: BoxFuture<'static, std::io::Result<()>>) -> std::io::Result<()> {
///         use futures::task::SpawnExt;
///
///         self.spawner
///             .spawn(async move {
///                 match fut.await {
///                     Ok(_) => {}
///                     Err(err) => {
///                         log::error!("IoSpawner catch err={}", err);
///                     }
///                 }
///             })
///             .map_err(|err| io::Error::new(io::ErrorKind::Other, err))
///     }
/// }
///
/// let mut local_pool = LocalPool::new();
///
/// register_local_spawner(MockIoSpawner {
///      spawner: local_pool.spawner(),
/// });
///
/// io_spawn(async { Ok(()) }).unwrap();
///
/// local_pool.run();
/// # }
/// ```
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
