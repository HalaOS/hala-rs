use std::{io, time::Duration};

use crate::{
    driver::{CtlOps, Description, Driver, Event, Handle},
    OpenOps,
};

pub struct Poller {
    driver: Driver,
    handle: Handle,
}

impl Clone for Poller {
    fn clone(&self) -> Self {
        Self {
            driver: self.driver.clone(),
            handle: self.driver.fd_clone(self.handle).unwrap(),
        }
    }
}

impl Poller {
    pub fn new(driver: Driver) -> io::Result<Self> {
        let handle = driver.fd_open(Description::Poller, OpenOps::None)?;

        Ok(Self { driver, handle })
    }

    pub fn poll_once(&self, timeout: Option<Duration>) -> io::Result<Vec<Event>> {
        self.driver
            .fd_ctl(self.handle, CtlOps::PollOnce(timeout))?
            .try_into_readiness()
    }

    pub fn raw_handle(&self) -> Handle {
        self.handle
    }
}

impl<'a> Into<Handle> for &'a Poller {
    fn into(self) -> Handle {
        self.raw_handle()
    }
}

impl Drop for Poller {
    fn drop(&mut self) {
        self.driver.fd_close(self.handle).unwrap()
    }
}
