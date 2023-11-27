use futures::{task::SpawnExt, Future};

use hala_reactor::*;

pub use futures;

pub use hala_io_test_derive::*;

#[allow(unused)]
mod mt {
    use std::thread::current;

    use super::*;
    pub fn spawner() -> &'static futures::executor::ThreadPool {
        static POOL: std::sync::OnceLock<futures::executor::ThreadPool> =
            std::sync::OnceLock::new();

        POOL.get_or_init(|| {
            #[cfg(feature = "debug")]
            pretty_env_logger::init();

            hala_reactor::mt::MioDevice::get().run_loop(None);

            log::trace!(
                "{:?} start mt::MioDevice run_loop successed",
                current().id()
            );

            futures::executor::ThreadPool::builder()
                .pool_size(20)
                .create()
                .unwrap()
        })
    }

    pub fn socket_tester<T, Fut>(label: &'static str, test: T)
    where
        T: FnOnce() -> Fut,
        Fut: Future + Send + 'static,
        Fut::Output: Send,
    {
        let thread_pool = spawner();

        log::trace!("start io test(mt,{})", label);

        let handle = thread_pool.spawn_with_handle(test()).unwrap();

        futures::executor::block_on(handle);
    }
}

#[allow(unused)]
mod st {
    use std::{cell::RefCell, sync::Once, thread::current, time::Duration};

    use futures::executor::block_on;

    use super::*;

    pub fn spawner() -> &'static futures::executor::ThreadPool {
        static POOL: std::sync::OnceLock<futures::executor::ThreadPool> =
            std::sync::OnceLock::new();

        POOL.get_or_init(|| {
            #[cfg(feature = "debug")]
            pretty_env_logger::init();

            futures::executor::ThreadPool::builder()
                .pool_size(1)
                .create()
                .unwrap()
        })
    }

    pub fn socket_tester<T, Fut>(label: &'static str, test: T)
    where
        T: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        static RUN_LOOP: Once = Once::new();

        RUN_LOOP.call_once(|| {
            #[cfg(feature = "debug")]
            {
                _ = pretty_env_logger::try_init();
            }

            let spawner_fn = move |fut| {
                spawner().spawn(fut).unwrap();
            };

            hala_reactor::st::MioDevice::run_loop(spawner_fn, None).unwrap();

            log::trace!(
                "{:?} start st::MioDevice run_loop successed",
                current().id()
            );
        });

        log::trace!("start io test(st,{})", label);

        let handle = spawner().spawn_with_handle(test()).unwrap();

        block_on(handle)
    }
}

#[cfg(feature = "mt")]
pub use mt::*;

#[cfg(all(not(feature = "mt"), feature = "st"))]
pub use st::*;
