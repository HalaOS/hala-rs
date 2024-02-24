use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
};

use hala_io::{
    context::{io_context, RawIoContext},
    *,
};

/// A UDP socket.
///
/// After creating a `UdpSocket` by [`bind`](Self::bind)ing it to a socket address, data can be [sent to](Self::send_to) and
/// [received from](Self::recv_from) any other socket address.
///
/// As stated in the User Datagram Protocol's specification in [IETF RFC 768], UDP is an unordered,
/// unreliable protocol. Refer to `TcpListener` and `TcpStream` for async TCP primitives.
///
/// This type is an async version of [`std::net::UdpSocket`].
///
pub struct UdpSocket {
    fd: Handle,
    driver: Driver,
}

impl UdpSocket {
    /// Creates a UDP socket from the given address.
    ///
    /// Binding with a port number of 0 will request that the OS assigns a port to this socket. The
    /// port allocated can be queried via the [`local_addr`](Self::local_addr) method.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # fn main() -> std::io::Result<()> { hala_future::executor::block_on(async {
    /// #
    /// use hala_udp::UdpSocket;
    ///
    /// let socket = UdpSocket::bind("127.0.0.1:0").await?;
    /// #
    /// # Ok(()) }) }
    /// ```
    pub async fn bind<S: ToSocketAddrs>(laddrs: S) -> io::Result<Self> {
        let io_context = io_context();

        let driver = io_context.driver().clone();
        let poller = io_context.poller();

        let laddrs = laddrs.to_socket_addrs()?.into_iter().collect::<Vec<_>>();

        let fd = driver.fd_open(Description::UdpSocket, OpenFlags::Bind(&laddrs))?;

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

        Ok(Self { fd, driver })
    }

    /// Returns the local address that this listener is bound to.
    ///
    /// This can be useful, for example, when binding to port 0 to figure out which port was
    /// actually bound.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # fn main() -> std::io::Result<()> { hala_future::executor::block_on(async {
    /// #
    /// use hala_udp::UdpSocket;
    ///
    /// let socket = UdpSocket::bind("127.0.0.1:0").await?;
    /// let addr = socket.local_addr()?;
    /// #
    /// # Ok(()) }) }
    /// ```
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.driver
            .fd_cntl(self.fd, Cmd::LocalAddr)?
            .try_into_sockaddr()
    }

    /// Sends data on the socket to the given address.
    ///
    /// On success, returns the number of bytes written.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # fn main() -> std::io::Result<()> { hala_future::executor::block_on(async {
    /// #
    /// use hala_udp::UdpSocket;
    ///
    /// const THE_MERCHANT_OF_VENICE: &[u8] = b"
    ///     If you prick us, do we not bleed?
    ///     If you tickle us, do we not laugh?
    ///     If you poison us, do we not die?
    ///     And if you wrong us, shall we not revenge?
    /// ";
    ///
    /// let socket = UdpSocket::bind("127.0.0.1:0").await?;
    ///
    /// let addr = "127.0.0.1:7878";
    /// let sent = socket.send_to(THE_MERCHANT_OF_VENICE, &addr).await?;
    /// println!("Sent {} bytes to {}", sent, addr);
    /// #
    /// # Ok(()) }) }
    /// ```
    pub async fn send_to<S: ToSocketAddrs>(&self, buf: &[u8], target: S) -> io::Result<usize> {
        let mut last_error = None;

        for raddr in target.to_socket_addrs()? {
            let result = would_block(|cx| {
                self.driver
                    .fd_cntl(
                        self.fd,
                        Cmd::SendTo {
                            waker: cx.waker().clone(),
                            buf,
                            raddr,
                        },
                    )?
                    .try_into_datalen()
            })
            .await;

            if result.is_ok() {
                return result;
            }

            last_error = Some(result);
        }

        last_error.unwrap()
    }

    /// Receives data from the socket.
    ///
    /// On success, returns the number of bytes read and the origin.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # fn main() -> std::io::Result<()> { hala_future::executor::block_on(async {
    /// #
    /// use hala_udp::UdpSocket;
    ///
    /// let socket = UdpSocket::bind("127.0.0.1:0").await?;
    ///
    /// let mut buf = vec![0; 1024];
    /// let (n, peer) = socket.recv_from(&mut buf).await?;
    /// println!("Received {} bytes from {}", n, peer);
    /// #
    /// # Ok(()) }) }
    /// ```
    pub async fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        would_block(|cx| {
            self.driver
                .fd_cntl(
                    self.fd,
                    Cmd::RecvFrom {
                        waker: cx.waker().clone(),
                        buf,
                    },
                )?
                .try_into_recv_from()
        })
        .await
    }

    /// Gets the value of the `SO_BROADCAST` option for this socket.
    ///
    /// For more information about this option, see [`set_broadcast`].
    ///
    /// [`set_broadcast`]: #method.set_broadcast
    pub async fn broadcast(&self) -> io::Result<bool> {
        would_block(|_| {
            self.driver
                .fd_cntl(self.fd, Cmd::BroadCast)?
                .try_into_broadcast()
        })
        .await
    }

    /// Sets the value of the `SO_BROADCAST` option for this socket.
    ///
    /// When enabled, this socket is allowed to send packets to a broadcast address.
    pub async fn set_broadcast(&self, on: bool) -> io::Result<()> {
        would_block(|_| {
            self.driver
                .fd_cntl(self.fd, Cmd::SetBroadCast(on))?
                .try_into_none()
        })
        .await
    }
}

impl Drop for UdpSocket {
    fn drop(&mut self) {
        self.driver.fd_close(self.fd).unwrap()
    }
}
