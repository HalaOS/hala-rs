use std::{fmt::Debug, io, net::ToSocketAddrs, task::Poll};

use futures::{AsyncRead, AsyncWrite};
use hala_io_driver::*;

pub struct TcpStream {
    fd: Handle,
    poller: Handle,
    driver: Driver,
}

impl Debug for TcpStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TcpStream({:?})", self.fd)
    }
}

impl TcpStream {
    pub fn new(driver: Driver, fd: Handle) -> io::Result<Self> {
        let poller = current_poller()?;

        match driver.fd_cntl(
            poller,
            Cmd::Register {
                source: fd,
                interests: Interest::Readable | Interest::Writable,
            },
        ) {
            Err(err) => {
                _ = driver.fd_close(fd);
                return Err(err);
            }
            _ => {}
        }

        Ok(Self { fd, driver, poller })
    }
    /// Opens a TCP connection to a remote host.
    pub fn connect<S: ToSocketAddrs>(raddrs: S) -> io::Result<Self> {
        let driver = get_driver()?;

        let raddrs = raddrs.to_socket_addrs()?.into_iter().collect::<Vec<_>>();

        let fd = driver.fd_open(Description::TcpStream, OpenFlags::Connect(&raddrs))?;

        Self::new(driver, fd)
    }
}

impl AsyncWrite for TcpStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        poll(|| {
            self.driver
                .fd_cntl(
                    self.fd,
                    Cmd::Write {
                        waker: cx.waker().clone(),
                        buf,
                    },
                )?
                .try_into_datalen()
        })
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

impl AsyncRead for TcpStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        poll(|| {
            self.driver
                .fd_cntl(
                    self.fd,
                    Cmd::Read {
                        waker: cx.waker().clone(),
                        buf,
                    },
                )?
                .try_into_datalen()
        })
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        self.driver
            .fd_cntl(self.poller, Cmd::Deregister(self.fd))
            .unwrap();
        self.driver.fd_close(self.fd).unwrap()
    }
}
