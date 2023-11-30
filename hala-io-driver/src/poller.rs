use std::{io, time::Duration};

use crate::driver::{CtlOps, Description, Driver, Event, Handle};

pub struct Poller {
    driver: Driver,
    handle: Handle,
}

impl Poller {
    pub fn new(driver: Driver) -> io::Result<Self> {
        let handle = driver.fd_open(Description::Poller, None)?;

        Ok(Self { driver, handle })
    }

    pub fn poll_once(&self, timeout: Option<Duration>) -> io::Result<Vec<Event>> {
        self.driver
            .fd_ctl(self.handle, CtlOps::PollOnce(timeout))?
            .try_into_readiness()
    }
}

impl Drop for Poller {
    fn drop(&mut self) {
        self.driver.fd_close(self.handle).unwrap()
    }
}
