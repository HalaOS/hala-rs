use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
};

use crate::driver::{CtlOps, Description, Driver, Handle, Interest, OpenOps, WriteOps};

pub struct TcpListener {
    pub driver: Driver,
    pub handle: Handle,
    pub poller: Handle,
}

impl TcpListener {
    /// Creates a new `TcpListener` which will be bound to the specified
    /// address.
    pub fn bind<P: Into<Handle>, S: ToSocketAddrs>(
        driver: &Driver,
        poller: P,
        laddrs: S,
    ) -> io::Result<Self> {
        let poller = poller.into();

        let laddrs = laddrs.to_socket_addrs()?.into_iter().collect::<Vec<_>>();

        let handle = driver.fd_open(Description::TcpListener, OpenOps::Bind(&laddrs))?;

        driver.fd_ctl(
            poller,
            CtlOps::Register {
                handles: &[handle],
                interests: Interest::Read,
            },
        )?;

        Ok(Self {
            handle,
            poller,
            driver: driver.clone(),
        })
    }

    pub fn accept(&self) -> io::Result<(TcpStream, SocketAddr)> {
        let (handle, raddr) = self
            .driver
            .fd_ctl(self.handle, CtlOps::Accept)?
            .try_into_incoming()?;

        Ok((TcpStream::new(&self.driver, self.poller, handle)?, raddr))
    }
}

impl Drop for TcpListener {
    fn drop(&mut self) {
        self.driver
            .fd_ctl(self.poller, CtlOps::Deregister(&[self.handle]))
            .unwrap();

        self.driver.fd_close(self.handle).unwrap();
    }
}

pub struct TcpStream {
    pub driver: Driver,
    pub poller: Handle,
    pub handle: Handle,
}

impl TcpStream {
    pub fn new(driver: &Driver, poller: Handle, handle: Handle) -> io::Result<Self> {
        driver.fd_ctl(
            poller,
            CtlOps::Register {
                handles: &[handle],
                interests: Interest::Read | Interest::Write,
            },
        )?;

        Ok(Self {
            driver: driver.clone(),
            handle,
            poller,
        })
    }

    pub fn connect<P: Into<Handle>, S: ToSocketAddrs>(
        driver: &Driver,
        poller: P,
        raddrs: S,
    ) -> io::Result<Self> {
        let poller = poller.into();

        let raddrs = raddrs.to_socket_addrs()?.into_iter().collect::<Vec<_>>();

        let handle = driver.fd_open(Description::TcpStream, OpenOps::Connect(&raddrs))?;

        Self::new(driver, poller, handle)
    }

    pub fn write(&self, buf: &[u8]) -> io::Result<usize> {
        self.driver.fd_write(self.handle, WriteOps::Write(buf))
    }

    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.driver.fd_read(self.handle, buf)?.try_to_read()
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        self.driver
            .fd_ctl(self.poller, CtlOps::Deregister(&[self.handle]))
            .unwrap();

        self.driver.fd_close(self.handle).unwrap();
    }
}

pub struct UdpSocket {
    pub driver: Driver,
    pub poller: Handle,
    pub handle: Handle,
}

impl UdpSocket {
    pub fn bind<P: Into<Handle>, S: ToSocketAddrs>(
        driver: &Driver,
        poller: P,
        laddrs: S,
    ) -> io::Result<Self> {
        let poller = poller.into();

        let laddrs = laddrs.to_socket_addrs()?.into_iter().collect::<Vec<_>>();

        let handle = driver.fd_open(Description::UdpSocket, OpenOps::Bind(&laddrs))?;

        driver.fd_ctl(
            poller,
            CtlOps::Register {
                handles: &[handle],
                interests: Interest::Read,
            },
        )?;

        Ok(Self {
            handle,
            poller,
            driver: driver.clone(),
        })
    }

    pub fn send_to(&self, buf: &[u8], raddr: SocketAddr) -> io::Result<usize> {
        self.driver
            .fd_write(self.handle, WriteOps::SendTo(buf, raddr))
    }

    pub fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        self.driver.fd_read(self.handle, buf)?.try_to_recv_from()
    }
}

impl Drop for UdpSocket {
    fn drop(&mut self) {
        self.driver
            .fd_ctl(self.poller, CtlOps::Deregister(&[self.handle]))
            .unwrap();

        self.driver.fd_close(self.handle).unwrap();
    }
}
