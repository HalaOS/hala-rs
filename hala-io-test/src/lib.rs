use std::{io, sync::Once};

use futures::{
    executor::{LocalPool, LocalSpawner},
    future::BoxFuture,
    Future,
};

use hala_io_driver::*;

pub use futures;

pub use hala_io_test_derive::*;

static START: Once = Once::new();

pub fn socket_tester<T, Fut>(label: &'static str, test: T)
where
    T: FnOnce() -> Fut + 'static,
    Fut: Future<Output = ()> + 'static,
{
    START.call_once(|| {
        _ = register_driver(mio_driver());
    });

    let mut local_pool = LocalPool::new();

    register_local_spawner(TesterIoSpawner {
        spawner: local_pool.spawner(),
    });

    let _guard = LocalPollerLoopGuard::new(None).unwrap();

    log::trace!("start io test(st,{})", label);

    local_pool.run_until(test());
}

struct TesterIoSpawner {
    spawner: LocalSpawner,
}

impl IoSpawner for TesterIoSpawner {
    fn spawn(&self, fut: BoxFuture<'static, std::io::Result<()>>) -> std::io::Result<()> {
        use futures::task::SpawnExt;

        self.spawner
            .spawn(async move {
                match fut.await {
                    Ok(_) => {}
                    Err(err) => {
                        log::error!("IoSpawner catch err={}", err);
                    }
                }
            })
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))
    }
}
