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

/// Get poller instance from global context.
pub fn get_poller() -> io::Result<Handle> {
    let poll = POLLER.get_or_init(|| {
        let driver = get_driver().unwrap();

        let handle = driver
            .fd_open(Description::Poller, OpenFlags::None)
            .unwrap();

        Poller(handle)
    });

    Ok(poll.0)
}

/// Get poller instance from local thread context.
pub fn get_local_poller() -> io::Result<Handle> {
    LOCAL_POLLER.with_borrow_mut(|poller| {
        if poller.is_none() {
            let driver = get_driver()?;

            let handle = driver.fd_open(Description::Poller, OpenFlags::None)?;

            *poller = Some(Poller(handle));
        }

        Ok(poller.as_ref().unwrap().0)
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

/// An RAII guard to control local thread `poller` event loop.
pub struct LocalPollLoopGuard {
    drop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl LocalPollLoopGuard {
    /// Create new `LocalPollerLoopGuard` with event loop `timeout`
    pub fn new(timeout: Option<Duration>) -> io::Result<Self> {
        let drop = Arc::new(AtomicBool::new(false));

        let drop_cloned = drop.clone();

        let driver = get_driver().unwrap();
        let poller = get_local_poller().unwrap();

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

impl Drop for LocalPollLoopGuard {
    fn drop(&mut self) {
        self.drop.store(true, Ordering::SeqCst);
        self.handle.take().unwrap().join().unwrap();
    }
}
