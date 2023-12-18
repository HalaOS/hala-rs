use std::cell::RefCell;

use futures::{future::BoxFuture, Future};

use std::{io, sync::OnceLock};

/// The `IoSpawner` trait allows for pushing an io futures onto an executor that will
/// run them to completion.
pub trait IoSpawner {
    /// Spawns a io task that polls the given future with output `io::Result<()>` to completion.
    fn spawn(&self, fut: BoxFuture<'static, io::Result<()>>) -> io::Result<()>;
}

thread_local! {
   pub(super) static LOCAL_SPAWNER: RefCell<Option<Box<dyn IoSpawner>>> = RefCell::new(None);
}

static SPAWNER: OnceLock<Box<dyn IoSpawner + Send + Sync>> = OnceLock::new();

/// Register [`IoSpawner`] for local thread.
pub fn register_local_spawner<S: IoSpawner + 'static>(spawner: S) {
    LOCAL_SPAWNER.set(Some(Box::new(spawner)));
}

/// Register [`IoSpawner`] for all thread. the `IoSpawner` instance must implement [`Send`] + [`Sync`] traits.
pub fn register_spawner<S: IoSpawner + Send + Sync + 'static>(spawner: S) {
    if let Err(_) = SPAWNER.set(Box::new(spawner)) {
        panic!("Call register_spawner more than once.");
    }
}

/// Spawn an io task that polls the given future with output `io::Result<()>` to completion.
/// ```
/// # {
/// use hala_io_driver::*;
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
    Fut: Future<Output = io::Result<()>> + Send + 'static,
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

#[cfg(test)]
mod tests {
    use std::io;

    use futures::executor::ThreadPool;
    use futures::task::SpawnExt;

    use crate::{io_spawn, register_spawner, IoSpawner};

    struct MockSpawner {
        pool: ThreadPool,
    }

    impl IoSpawner for MockSpawner {
        fn spawn(
            &self,
            fut: futures::prelude::future::BoxFuture<'static, std::io::Result<()>>,
        ) -> std::io::Result<()> {
            self.pool
                .spawn(async {
                    match fut.await {
                        Ok(_) => {}
                        Err(err) => {
                            log::error!("MockSpawner catch err={}", err);
                        }
                    }
                })
                .map_err(|err| io::Error::new(io::ErrorKind::Other, err))
        }
    }

    #[test]
    fn test_register_spawner() {
        register_spawner(MockSpawner {
            pool: ThreadPool::new().unwrap(),
        });

        io_spawn(async { Ok(()) }).unwrap();
    }
}
