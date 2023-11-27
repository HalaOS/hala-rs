use futures::{
    executor::{block_on, LocalPool, LocalSpawner},
    task::{LocalSpawnExt, SpawnExt},
    Future,
};

use hala_reactor::*;

pub use futures;

pub use hala_io_test_derive::*;

#[allow(unused)]
mod mt {
    use super::*;
    pub fn spawner() -> &'static futures::executor::ThreadPool {
        static POOL: std::sync::OnceLock<futures::executor::ThreadPool> =
            std::sync::OnceLock::new();

        POOL.get_or_init(|| {
            #[cfg(feature = "debug")]
            pretty_env_logger::init();

            futures::executor::ThreadPool::builder()
                .pool_size(20)
                .create()
                .unwrap()
        })
    }

    pub fn socket_tester<T, Fut>(test: T)
    where
        T: FnOnce() -> Fut,
        Fut: Future + Send + 'static,
        Fut::Output: Send,
    {
        hala_reactor::MioDeviceMT::get().run_loop(None);

        let thread_pool = spawner();

        let handle = thread_pool.spawn_with_handle(test()).unwrap();

        futures::executor::block_on(handle);
    }
}

#[allow(unused)]
mod st {
    use super::*;

    pub fn spawner() -> LocalSpawner {
        thread_local! {
            static POOL: LocalPool = LocalPool::new();
        }

        POOL.with(|pool| pool.spawner())
    }

    // #[cfg(all(not(feature = "mt"), feature = "st"))]
    pub fn socket_tester<T, Fut>(test: T)
    where
        T: FnOnce() -> Fut,
        Fut: Future + Send + 'static,
        Fut::Output: Send,
    {
        hala_reactor::MioDeviceST::run_loop(
            &move |fut| {
                spawner().spawn_local(fut).unwrap();
            },
            None,
        )
        .unwrap();

        let spawner = spawner();
        let fut = spawner.spawn_with_handle(test()).unwrap();

        block_on(fut);
    }
}

#[cfg(feature = "mt")]
pub use mt::*;

#[cfg(all(not(feature = "mt"), feature = "st"))]
pub use st::*;
