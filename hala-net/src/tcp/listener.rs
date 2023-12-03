use std::{
    fmt::Debug,
    io,
    net::{SocketAddr, ToSocketAddrs},
};

use hala_io_driver::*;

use super::TcpStream;

/// A structure representing a socket tcp server
pub struct TcpListener {
    fd: Handle,
    driver: Driver,
}

impl Debug for TcpListener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TcpListener(Handle = {:?})", self.fd)
    }
}

impl TcpListener {
    /// Create new tcp listener with calling underly bind method.
    pub fn bind<S: ToSocketAddrs>(laddrs: S) -> io::Result<Self> {
        let driver = get_driver()?;

        let laddrs = laddrs.to_socket_addrs()?.into_iter().collect::<Vec<_>>();

        let fd = driver.fd_open(Description::TcpListener, OpenFlags::Bind(&laddrs))?;

        let poller = current_poller()?;

        match driver.fd_cntl(
            poller,
            Cmd::Register {
                source: fd,
                interests: Interest::Readable,
            },
        ) {
            Err(err) => {
                _ = driver.fd_close(fd);
                return Err(err);
            }
            _ => {}
        }

        Ok(Self { fd, driver })
    }

    /// Accepts a new incoming connection from this listener.
    pub async fn accept(&self) -> io::Result<(TcpStream, SocketAddr)> {
        let (handle, raddr) = async_io(|cx| {
            self.driver
                .fd_cntl(self.fd, Cmd::Accept(cx.waker().clone()))
        })
        .await?
        .try_into_incoming()?;

        Ok((TcpStream::new(self.driver.clone(), handle)?, raddr))
    }

    /// Returns the local socket address of this listener.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.driver
            .fd_cntl(self.fd, Cmd::LocalAddr)?
            .try_into_sockaddr()
    }
}

impl Drop for TcpListener {
    fn drop(&mut self) {
        self.driver.fd_close(self.fd).unwrap()
    }
}
