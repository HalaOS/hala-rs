use hala_io_driver::{Description, Handle, OpenFlags};

use super::*;

use std::{cell::RefCell, io, sync::OnceLock};

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
    let poller = POLLER.get_or_init(|| {
        let driver = get_driver().expect("call register_driver first");

        let handle = driver
            .fd_open(Description::Poller, OpenFlags::None)
            .unwrap();

        Poller(handle)
    });

    Ok(poller.0)
}

/// Get poller instance from local thread context.
pub fn get_local_poller() -> io::Result<Handle> {
    LOCAL_POLLER.with_borrow_mut(|poller| {
        if poller.is_none() {
            let driver = get_driver()?;

            let handle = driver.fd_open(Description::Poller, OpenFlags::LocalPoller)?;

            *poller = Some(Poller(handle));
        }

        Ok(poller.as_ref().unwrap().0)
    })
}

pub trait ContextPoller {
    /// Get poller handle from context.
    fn get() -> io::Result<Handle>;
}

/// Structure to get global context poller
pub struct GlobalContextPoller;

/// Structure to get local thread context poller
pub struct LocalContextPoller;

impl ContextPoller for GlobalContextPoller {
    fn get() -> io::Result<Handle> {
        get_poller()
    }
}

impl ContextPoller for LocalContextPoller {
    fn get() -> io::Result<Handle> {
        get_local_poller()
    }
}
