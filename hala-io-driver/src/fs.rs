use std::io;

use crate::driver::{CtlOps, Description, Driver, Handle, Interest, OpenOps, WriteOps};

pub struct File {
    pub driver: Driver,
    pub handle: Handle,
    pub poller: Handle,
}

impl File {
    pub fn new<P: Into<Handle>>(
        driver: &Driver,
        poller: P,
        path: &str,
        interests: Interest,
    ) -> io::Result<Self> {
        let poller = poller.into();
        let handle = driver.fd_open(Description::File, OpenOps::OpenFile(path))?;

        driver.fd_ctl(
            poller,
            CtlOps::Register {
                handles: &[handle],
                interests,
            },
        )?;

        Ok(Self {
            handle,
            poller,
            driver: driver.clone(),
        })
    }

    pub fn write(&self, buf: &[u8]) -> io::Result<usize> {
        self.driver.fd_write(self.handle, WriteOps::Write(buf))
    }

    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.driver.fd_read(self.handle, buf)?.try_to_read()
    }
}

impl Drop for File {
    fn drop(&mut self) {
        self.driver
            .fd_ctl(self.poller, CtlOps::Deregister(&[self.handle]))
            .unwrap();

        self.driver.fd_close(self.handle).unwrap();
    }
}
