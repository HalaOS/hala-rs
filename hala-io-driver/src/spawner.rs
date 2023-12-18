use std::{cell::RefCell, io, sync::OnceLock};

use futures::{future::BoxFuture, Future};

pub trait IoSpawner {
    fn spawn(&self, fut: BoxFuture<'static, io::Result<()>>) -> io::Result<()>;
}

thread_local! {
    static LOCAL_SPAWNER: RefCell<Option<Box<dyn IoSpawner>>> = RefCell::new(None);
}

static SPAWNER: OnceLock<Box<dyn IoSpawner + Send + Sync>> = OnceLock::new();

/// Register local thread `Spawner`
pub fn register_local_io_spawner<S: IoSpawner + 'static>(spawner: S) {
    LOCAL_SPAWNER.set(Some(Box::new(spawner)));
}

/// Register local thread `Spawner`
pub fn register_io_spawner<S: IoSpawner + Send + Sync + 'static>(spawner: S) {
    if let Err(_) = SPAWNER.set(Box::new(spawner)) {
        panic!("Call register_spawner more than once.");
    }
}

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
