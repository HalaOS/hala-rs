use std::{cell::RefCell, io, time::Duration};

use crate::{CtlOps, Description, Driver, Handle, Interest, OpenOps};

pub struct Tick {
    driver: Driver,
    poller: Handle,
    handle: Handle,
    times: RefCell<usize>,
}

impl Tick {
    pub fn new<P: Into<Handle>>(
        driver: &Driver,
        poller: P,
        duration: Duration,
        oneshot: bool,
    ) -> io::Result<Tick> {
        let poller = poller.into();

        let handle = driver.fd_open(Description::Timeout, OpenOps::Tick { duration, oneshot })?;

        driver.fd_ctl(
            poller,
            CtlOps::Register {
                handles: &[handle],
                interests: Interest::Read,
            },
        )?;

        Ok(Self {
            driver: driver.clone(),
            handle,
            poller,
            times: Default::default(),
        })
    }

    /// If timeout expired returns Ok(()), otherwise returns error `WOULD_BLOCK`
    pub fn read(&self) -> io::Result<()> {
        *self.times.borrow_mut() = self
            .driver
            .fd_ctl(self.handle, CtlOps::Tick(*self.times.borrow()))?
            .try_into_tick()?;

        Ok(())
    }
}

impl Drop for Tick {
    fn drop(&mut self) {
        self.driver
            .fd_ctl(self.poller, CtlOps::Deregister(&[self.handle]))
            .unwrap();

        self.driver.fd_close(self.handle).unwrap();
    }
}
