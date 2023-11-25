use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
    task::Poll,
};

use hala_reactor::{each_addr, global_io_device, IoObject, ToIoObject};
use mio::Interest;

/// A UDP socket.
pub struct UdpSocket {
    io: IoObject<mio::net::UdpSocket>,
}

impl UdpSocket {
    /// This function will create a new UDP socket and attempt to bind it to the addr provided.
    pub async fn bind<S: ToSocketAddrs>(laddr: S) -> io::Result<Self> {
        let sys_udp_socket = std::net::UdpSocket::bind(laddr)?;

        sys_udp_socket.set_nonblocking(true)?;

        let socket = mio::net::UdpSocket::from_std(sys_udp_socket);

        Ok(Self {
            io: IoObject::new(
                global_io_device().clone(),
                socket,
                Interest::READABLE.add(Interest::WRITABLE),
            )?,
        })
    }

    /// Returns the local address that this socket is bound to.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.io.inner_object.get().local_addr()
    }

    /// Connects the UDP socket setting the default destination for `send()`
    /// and limiting packets that are read via `recv` from the address specified
    /// in `addr`.
    pub async fn connect<S: ToSocketAddrs>(&self, remote: S) -> io::Result<()> {
        let call = |addr| {
            self.io.async_io(Interest::WRITABLE, move || {
                self.io.inner_object.get().connect(addr)
            })
        };

        each_addr(remote, call).await
    }

    /// Sends data on the socket to the given address. On success, returns the
    /// number of bytes written.
    pub async fn send_to<S: ToSocketAddrs>(&self, buf: &[u8], target: S) -> io::Result<usize> {
        let call = |addr| {
            self.io.async_io(Interest::WRITABLE, move || {
                self.io.inner_object.get().send_to(buf, addr)
            })
        };

        each_addr(target, call).await
    }

    /// Receives data from the socket. On success, returns the number of bytes
    /// read and the address from whence the data came.
    pub async fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        self.io
            .async_io(Interest::WRITABLE, move || {
                self.io.inner_object.get().recv_from(buf)
            })
            .await
    }

    pub fn poll_recv_from(
        &self,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<(usize, SocketAddr)>> {
        self.io.poll_io(cx, Interest::READABLE, || {
            self.io.inner_object.get().recv_from(buf)
        })
    }
}

impl ToIoObject<mio::net::UdpSocket> for UdpSocket {
    fn to_io_object(&self) -> &IoObject<mio::net::UdpSocket> {
        &self.io
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[hala_io_test::test]
    async fn test_udp() {
        let send_buf = b"hello world";

        let server_udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();

        let client_udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();

        client_udp
            .send_to(send_buf, server_udp.local_addr().unwrap())
            .await
            .unwrap();

        let mut buf = [0 as u8; 1024];

        let (recv_size, remote_addr) = server_udp.recv_from(&mut buf).await.unwrap();

        assert_eq!(recv_size, send_buf.len());

        assert_eq!(remote_addr, client_udp.local_addr().unwrap());
    }
}
