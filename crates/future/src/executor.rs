use std::sync::OnceLock;

use futures::{future::BoxFuture, Future, FutureExt};

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
