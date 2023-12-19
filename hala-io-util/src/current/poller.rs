use hala_io_driver::{Cmd, Description, Handle, OpenFlags};

use super::*;

use std::{
    cell::RefCell,
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
    thread::JoinHandle,
    time::Duration,
};

thread_local! {
    static LOCAL_POLLER: RefCell<Option<Poller>> = RefCell::new(None);
}

static POLLER: OnceLock<Poller> = OnceLock::new();

struct Poller(Handle);

impl Drop for Poller {
    fn drop(&mut self) {
        get_driver().unwrap().fd_close(self.0).unwrap();
    }
}

pub fn register_local_poller() -> io::Result<()> {
    LOCAL_POLLER.with_borrow_mut(|poller| {
        let driver = get_driver()?;

        let handle = driver.fd_open(Description::Poller, OpenFlags::None)?;

        *poller = Some(Poller(handle));

        Ok(())
    })
}

pub fn register_poller() -> io::Result<()> {
    let driver = get_driver().expect("call register_driver first");

    let handle = driver
        .fd_open(Description::Poller, OpenFlags::None)
        .unwrap();

    if let Err(_) = POLLER.set(Poller(handle)) {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "Call register_poller more than once",
        ));
    }

    Ok(())
}

/// Get poller instance from global context.
pub fn get_poller() -> io::Result<Handle> {
    LOCAL_POLLER.with_borrow(|spawner| {
        if let Some(poller) = spawner {
            return Ok(poller.0);
        }

        if let Some(poller) = POLLER.get() {
            return Ok(poller.0);
        }

        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Call register_local_poller / register_poller first",
        ));
    })
}

/// Get poller instance from local thread context.
pub fn get_local_poller() -> io::Result<Handle> {
    LOCAL_POLLER.with_borrow_mut(|poller| {
        let poller = poller.as_ref().ok_or(io::Error::new(
            io::ErrorKind::NotFound,
            "Call register_local_poller first",
        ))?;

        Ok(poller.0)
    })
}

/// An RAII guard to control global context `poller` event loop.
pub struct PollLoopGuard {
    drop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl PollLoopGuard {
    /// Create new `PollerLoopGuard` with event loop `timeout`
    pub fn new(timeout: Option<Duration>) -> io::Result<Self> {
        let drop = Arc::new(AtomicBool::new(false));

        let drop_cloned = drop.clone();

        let driver = get_driver().unwrap();
        let poller = get_poller().unwrap();

        let handle = std::thread::spawn(move || {
            while !drop_cloned.load(Ordering::SeqCst) {
                // log::trace!("[PollGuard] poll_once");
                driver.fd_cntl(poller, Cmd::PollOnce(timeout)).unwrap();
            }
        });

        Ok(Self {
            drop,
            handle: Some(handle),
        })
    }
}

impl Drop for PollLoopGuard {
    fn drop(&mut self) {
        self.drop.store(true, Ordering::SeqCst);

        self.handle.take().unwrap().join().unwrap();
    }
}
