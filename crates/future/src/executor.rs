use std::sync::OnceLock;

use futures::{executor::ThreadPool, future::BoxFuture, task::SpawnExt, Future, FutureExt};

/// Future executor must implement this trait to support register to hala register system.
pub trait FutureSpawner {
    /// The implementation must panic if this function spawn future failed.
    fn spawn_boxed_future(&self, future: BoxFuture<'static, ()>);
}

static REGISTER: OnceLock<Box<dyn FutureSpawner + Send + Sync + 'static>> = OnceLock::new();

/// Register global spawner implementation.
pub fn register_spawner<S: FutureSpawner + Send + Sync + 'static>(spawner: S) {
    if REGISTER.set(Box::new(spawner)).is_err() {
        panic!("Call register_spawner twice.");
    }
}

/// Using global register [`Spawner`] to start a new future task.
pub fn future_spawn<Fut>(fut: Fut)
where
    Fut: Future<Output = ()> + Send + 'static,
{
    let spawner = REGISTER.get_or_init(|| {
        Box::new(
            ThreadPool::builder()
                .pool_size(num_cpus::get())
                .create()
                .unwrap(),
        )
    });

    spawner.spawn_boxed_future(fut.boxed())
}

impl FutureSpawner for futures::executor::ThreadPool {
    fn spawn_boxed_future(&self, future: BoxFuture<'static, ()>) {
        self.spawn(future)
            .expect("futures::executor::ThreadPool spawn failed");
    }
}

pub fn block_on<Fut, R>(fut: Fut) -> R
where
    Fut: Future<Output = R> + Send + 'static,
    R: Send + 'static,
{
    let (sender, receiver) = futures::channel::oneshot::channel::<R>();

    future_spawn(async move {
        let r = fut.await;
        _ = sender.send(r);
    });

    futures::executor::block_on(async move { receiver.await.unwrap() })
}
