use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
    task::Poll,
};

use hala_io_driver::*;

/// A UDP socket.
pub struct UdpSocket {
    fd: Handle,
    driver: Driver,
}

impl UdpSocket {
    /// This function will create a new UDP socket and attempt to bind it to the addr provided.
    pub async fn bind<S: ToSocketAddrs>(laddrs: S) -> io::Result<Self> {
        let driver = get_driver()?;

        let laddrs = laddrs.to_socket_addrs()?.into_iter().collect::<Vec<_>>();

        let fd = driver.fd_open(Description::UdpSocket, OpenFlags::Bind(&laddrs))?;

        Ok(Self { fd, driver })
    }

    /// Returns the local address that this socket is bound to.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        todo!()
    }

    /// Sends data on the socket to the given address. On success, returns the
    /// number of bytes written.
    pub async fn send_to<S: ToSocketAddrs>(&self, buf: &[u8], target: S) -> io::Result<usize> {
        todo!()
    }

    /// Receives data from the socket. On success, returns the number of bytes
    /// read and the address from whence the data came.
    pub async fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        todo!()
    }

    pub fn poll_recv_from(
        &self,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<(usize, SocketAddr)>> {
        todo!()
    }
}
