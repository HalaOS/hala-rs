use std::{
    fmt::Debug,
    io,
    net::{SocketAddr, ToSocketAddrs},
};

use hala_io::{
    context::{io_context, RawIoContext},
    *,
};

use super::TcpStream;

/// A TCP socket server, listening for connections.
///
/// After creating a `TcpListener` by [`bind`]ing it to a socket address, it listens for incoming
/// TCP connections. These can be accepted by awaiting elements from the async stream of
/// `incoming` connections.
///
/// The socket will be closed when the value is dropped.
///
/// The Transmission Control Protocol is specified in [IETF RFC 793].
///
/// This type is an async version of [`std::net::TcpListener`].
///
/// [`bind`]: #method.bind
/// [IETF RFC 793]: https://tools.ietf.org/html/rfc793
/// [`std::net::TcpListener`]: https://doc.rust-lang.org/std/net/struct.TcpListener.html
///
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
    /// Create new `TcpListener` which will be bound to the specified `laddrs`
    ///
    /// The returned listener is ready for accepting connections.
    ///
    /// Binding with a port number of 0 will request that the OS assigns a port to this listener.
    /// The port allocated can be queried via the [`local_addr`](TcpListener::local_addr) method.
    ///
    /// # Examples
    /// Create a TCP listener bound to 127.0.0.1:0:
    ///
    /// ```no_run
    /// # fn main() -> std::io::Result<()> { hala_future::executor::block_on(async {
    /// #
    /// use hala_tcp::TcpListener;
    ///
    /// let listener = TcpListener::bind("127.0.0.1:0").await?;
    /// #
    /// # Ok(()) }) }
    /// ```
    pub async fn bind<S: ToSocketAddrs>(laddrs: S) -> io::Result<Self> {
        let io_context = io_context();

        let driver = io_context.driver().clone();
        let poller = io_context.poller();

        let laddrs = laddrs.to_socket_addrs()?.into_iter().collect::<Vec<_>>();

        let fd = driver.fd_open(Description::TcpListener, OpenFlags::Bind(&laddrs))?;

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

    /// Accepts a new incoming connection to this listener.
    ///
    /// When a connection is established, the corresponding stream and address will be returned.
    ///
    /// ## Examples
    ///
    /// ```no_run
    /// # fn main() -> std::io::Result<()> { hala_future::executor::block_on(async {
    /// #
    /// use hala_tcp::TcpListener;
    ///
    /// let listener = TcpListener::bind("127.0.0.1:0").await?;
    /// let (stream, addr) = listener.accept().await?;
    /// #
    /// # Ok(()) }) }
    /// ```
    pub async fn accept(&self) -> io::Result<(TcpStream, SocketAddr)> {
        let (handle, raddr) = would_block(|cx| {
            self.driver
                .fd_cntl(self.fd, Cmd::Accept(cx.waker().clone()))
        })
        .await?
        .try_into_incoming()?;

        let stream = TcpStream::new_with(self.driver.clone(), handle, io_context().poller())?;

        log::trace!("tcp incoming token={:?}, raddr={}", handle.token, raddr);

        Ok((stream, raddr))
    }

    /// Returns the local address that this listener is bound to.
    ///
    /// This can be useful, for example, to identify when binding to port 0 which port was assigned
    /// by the OS.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # fn main() -> std::io::Result<()> { hala_future::executor::block_on(async {
    /// #
    /// use hala_tcp::TcpListener;
    ///
    /// let listener = TcpListener::bind("127.0.0.1:8080").await?;
    /// let addr = listener.local_addr()?;
    /// #
    /// # Ok(()) }) }
    /// ```
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
