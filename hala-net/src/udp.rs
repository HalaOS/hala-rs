use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
    task::Poll,
};

use hala_reactor::*;
use mio::Interest;

/// A UDP socket.
pub struct UdpSocket<IO: IoDevice + ContextIoDevice + 'static = MioDeviceMT> {
    io: IoObject<IO, mio::net::UdpSocket>,
}

impl<IO> UdpSocket<IO>
where
    IO: IoDevice + ContextIoDevice + Send + Sync + 'static,
{
    /// This function will create a new UDP socket and attempt to bind it to the addr provided.
    pub async fn bind<S: ToSocketAddrs>(laddr: S) -> io::Result<Self> {
        let sys_udp_socket = std::net::UdpSocket::bind(laddr)?;

        sys_udp_socket.set_nonblocking(true)?;

        let socket = mio::net::UdpSocket::from_std(sys_udp_socket);

        Ok(Self {
            io: IoObject::new(socket, Interest::READABLE.add(Interest::WRITABLE))?,
        })
    }

    /// Returns the local address that this socket is bound to.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.io.holder.get().local_addr()
    }

    /// Connects the UDP socket setting the default destination for `send()`
    /// and limiting packets that are read via `recv` from the address specified
    /// in `addr`.
    pub async fn connect<S: ToSocketAddrs>(&self, remote: S) -> io::Result<()> {
        let io = IO::get();

        let call = |addr| {
            self.io.async_io(&io, Interest::WRITABLE, move || {
                self.io.holder.get().connect(addr)
            })
        };

        each_addr(remote, call).await
    }

    /// Sends data on the socket to the given address. On success, returns the
    /// number of bytes written.
    pub async fn send_to<S: ToSocketAddrs>(&self, buf: &[u8], target: S) -> io::Result<usize> {
        let io = IO::get();

        let call = |addr| {
            self.io.async_io(&io, Interest::WRITABLE, move || {
                self.io.holder.get().send_to(buf, addr)
            })
        };

        each_addr(target, call).await
    }

    /// Receives data from the socket. On success, returns the number of bytes
    /// read and the address from whence the data came.
    pub async fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        let io = IO::get();

        self.io
            .async_io(&io, Interest::READABLE, move || {
                self.io.holder.get().recv_from(buf)
            })
            .await
    }

    pub fn poll_recv_from(
        &self,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<(usize, SocketAddr)>> {
        let io = IO::get();

        self.io.poll_io(&io, cx, Interest::READABLE, || {
            self.io.holder.get().recv_from(buf)
        })
    }
}

#[cfg(test)]
mod tests {
    use futures::task::SpawnExt;
    use rand::seq::SliceRandom;

    use super::*;

    #[test]
    fn test_udp_mt() {
        // pretty_env_logger::init();

        MioDeviceMT::get().start(None);
        async fn test_udp() {
            let send_buf = b"hello world";

            let server_udp: UdpSocket = UdpSocket::bind("127.0.0.1:0").await.unwrap();

            let client_udp: UdpSocket = UdpSocket::bind("127.0.0.1:0").await.unwrap();

            client_udp
                .send_to(send_buf, server_udp.local_addr().unwrap())
                .await
                .unwrap();

            log::trace!("send to");

            let mut buf = [0 as u8; 1024];

            let (recv_size, remote_addr) = server_udp.recv_from(&mut buf).await.unwrap();

            log::trace!("recv from");

            assert_eq!(recv_size, send_buf.len());

            assert_eq!(remote_addr, client_udp.local_addr().unwrap());
        }

        let pool = futures::executor::ThreadPool::builder()
            .pool_size(10)
            .create()
            .unwrap();

        let remote = pool.spawn_with_handle(test_udp()).unwrap();

        futures::executor::block_on(remote);
    }

    #[test]
    fn test_udp_select_mt() {
        // _ = pretty_env_logger::try_init();

        MioDeviceMT::get().start(None);

        async fn test_udp() {
            let send_buf = b"hello world";

            let mut server_udps: Vec<UdpSocket> = vec![];

            for _ in 0..10 {
                server_udps.push(UdpSocket::bind("127.0.0.1:0").await.unwrap());
            }

            let client_udp: UdpSocket = UdpSocket::bind("127.0.0.1:0").await.unwrap();

            for _ in 0..1000 {
                let server_udp = server_udps.choose(&mut rand::thread_rng()).unwrap();

                client_udp
                    .send_to(send_buf, server_udp.local_addr().unwrap())
                    .await
                    .unwrap();

                log::trace!("send to");

                let mut buf = [0 as u8; 1024];

                let (recv_size, remote_addr) = server_udp.recv_from(&mut buf).await.unwrap();

                log::trace!("recv from");

                assert_eq!(recv_size, send_buf.len());

                assert_eq!(remote_addr, client_udp.local_addr().unwrap());
            }
        }

        let pool = futures::executor::ThreadPool::builder()
            .pool_size(10)
            .create()
            .unwrap();

        let remote = pool.spawn_with_handle(test_udp()).unwrap();

        futures::executor::block_on(remote);
    }
}
