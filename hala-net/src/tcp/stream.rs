use std::{
    fmt::Debug,
    io::{self, Read, Write},
    net::ToSocketAddrs,
    task::Poll,
};

use futures::{AsyncRead, AsyncWrite};
use hala_io_driver::*;

pub struct TcpStream {
    fd: Handle,
    driver: Driver,
}

impl Debug for TcpStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TcpStream({:?})", self.fd)
    }
}

impl TcpStream {
    /// Opens a TCP connection to a remote host.
    pub async fn connect<S: ToSocketAddrs>(raddrs: S) -> io::Result<Self> {
        let driver = get_driver()?;

        let raddrs = raddrs.to_socket_addrs()?.into_iter().collect::<Vec<_>>();

        let fd = driver.fd_open(Description::TcpStream, OpenFlags::Connect(&raddrs))?;

        Ok(Self { fd, driver })
    }
}

impl AsyncWrite for TcpStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        todo!()
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        todo!()
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        todo!()
    }
}

impl AsyncRead for TcpStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        todo!()
    }
}
