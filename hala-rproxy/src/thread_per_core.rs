use std::{cell::RefCell, io, rc::Rc};

use futures::{
    executor::{LocalPool, LocalSpawner},
    future::BoxFuture,
    task::{LocalSpawnExt, SpawnExt},
    Future,
};
use hala_io_util::{
    get_driver, get_local_poller, register_local_poller, register_local_spawner, IoSpawner,
};
use hala_net::driver::Cmd;

///	Run a hala io future to completion on the current thread.
pub fn thread_per_core_block_on<Fut>(fut: Fut) -> io::Result<()>
where
    Fut: Future<Output = io::Result<()>> + Send + 'static,
{
    register_local_poller()?;

    let mut pool = LocalPool::new();

    register_local_spawner(ThreadPerCoreSpawner(pool.spawner()));

    let dropping = Rc::new(RefCell::new(false));

    let dropping_cloned = dropping.clone();

    let start_fut = async move {
        match fut.await {
            Ok(()) => {
                log::trace!("thread_per_core exit: OK(())")
            }
            Err(err) => {
                log::error!("thread_per_core exit: {}", err)
            }
        }

        *dropping_cloned.borrow_mut() = true;
    };

    pool.spawner().spawn_local(start_fut).map_err(|err| {
        io::Error::new(io::ErrorKind::Other, format!("Spawn local error: {}", err))
    })?;

    let driver = get_driver()?;
    let poller = get_local_poller()?;

    while !*dropping.borrow() {
        driver.fd_cntl(poller, Cmd::PollOnce(None)).unwrap();
        pool.run_until_stalled();
    }

    Ok(())
}

struct ThreadPerCoreSpawner(LocalSpawner);

impl IoSpawner for ThreadPerCoreSpawner {
    fn spawn(&self, fut: BoxFuture<'static, std::io::Result<()>>) -> std::io::Result<()> {
        self.0
            .spawn(async move {
                match fut.await {
                    Ok(_) => {}
                    Err(err) => {
                        log::error!("ThreadPerCoreSpawner catch err={}", err);
                    }
                }
            })
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))
    }
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use hala_io_util::{register_driver, sleep};
    use hala_net::driver::mio_driver;

    use super::*;

    #[test]
    fn test_thread_per_core_block_on() {
        register_driver(mio_driver()).unwrap();

        thread_per_core_block_on(async {
            sleep(Duration::from_secs(1)).await?;

            Ok(())
        })
        .unwrap();
    }
}
