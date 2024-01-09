use std::sync::OnceLock;

use futures::{executor::ThreadPool, future::BoxFuture, task::SpawnExt, Future, FutureExt};

/// Future executor must implement this trait to support register to hala register system.
pub trait Spawner {
    /// The implementation must panic if this function spawn future failed.
    fn spawn_boxed_future(&self, future: BoxFuture<'static, ()>);
}

static REGISTER: OnceLock<Box<dyn Spawner + Send + Sync + 'static>> = OnceLock::new();

/// Register global spawner implementation.
pub fn register_spawner<S: Spawner + Send + Sync + 'static>(spawner: S) {
    if REGISTER.set(Box::new(spawner)).is_err() {
        panic!("Call register_spawner twice.");
    }
}

/// Using global register [`Spawner`] to start a new future task.
pub fn spawn<Fut>(fut: Fut)
where
    Fut: Future<Output = ()> + Send + 'static,
{
    REGISTER.get().unwrap().spawn_boxed_future(fut.boxed())
}

impl Spawner for futures::executor::ThreadPool {
    fn spawn_boxed_future(&self, future: BoxFuture<'static, ()>) {
        self.spawn(future)
            .expect("futures::executor::ThreadPool spawn failed");
    }
}

pub fn block_on<Fut, R>(fut: Fut, pool_size: usize) -> R
where
    Fut: Future<Output = R> + Send + 'static,
    R: Send + 'static,
{
    static POOL: OnceLock<ThreadPool> = OnceLock::new();

    let pool = POOL.get_or_init(|| {
        let pool = ThreadPool::builder().pool_size(pool_size).create().unwrap();

        register_spawner(pool.clone());

        pool
    });

    let handle = pool
        .spawn_with_handle(fut)
        .expect("futures::executor::ThreadPool spawn enter future failed.");

    futures::executor::block_on(handle)
}
