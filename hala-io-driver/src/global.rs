use super::*;

use std::{
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
    thread::JoinHandle,
    time::Duration,
};

static INSTANCE_DRIVER: OnceLock<Driver> = OnceLock::new();

/// Get the currently registered io driver, or return a NotFound error if it is not registered.
pub fn get_driver() -> io::Result<Driver> {
    return INSTANCE_DRIVER
        .get()
        .map(|driver| driver.clone())
        .ok_or(io::Error::new(
            io::ErrorKind::NotFound,
            "[Hala-IO] call register_local_driver/register_driver first",
        ));
}

/// Register new io driver
pub fn register_driver<D: Into<Driver>>(driver: D) -> io::Result<()> {
    INSTANCE_DRIVER.set(driver.into()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "[Hala-IO] call register_driver twice",
        )
    })
}

struct Poll {
    driver: Driver,
    handle: Handle,
}

impl Drop for Poll {
    fn drop(&mut self) {
        self.driver.fd_close(self.handle).unwrap();
    }
}

static INSTANCE_POLL: OnceLock<Poll> = OnceLock::new();

pub fn current_poller() -> io::Result<Handle> {
    let poll = INSTANCE_POLL.get_or_init(|| {
        let driver = get_driver().unwrap();

        let handle = driver
            .fd_open(Description::Poller, OpenFlags::None)
            .unwrap();

        Poll { driver, handle }
    });

    Ok(poll.handle)
}

pub struct PollGuard {
    drop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl PollGuard {
    pub fn new(timeout: Option<Duration>) -> io::Result<Self> {
        let drop = Arc::new(AtomicBool::new(false));

        let drop_cloned = drop.clone();

        let driver = get_driver().unwrap();
        let poller = current_poller().unwrap();

        let handle = std::thread::spawn(move || {
            while !drop_cloned.load(Ordering::SeqCst) {
                log::trace!("[PollGuard] poll_once");
                driver.fd_cntl(poller, Cmd::PollOnce(timeout)).unwrap();
            }
        });

        Ok(Self {
            drop,
            handle: Some(handle),
        })
    }
}

impl Drop for PollGuard {
    fn drop(&mut self) {
        self.drop.store(true, Ordering::SeqCst);

        self.handle.take().unwrap().join().unwrap();
    }
}
