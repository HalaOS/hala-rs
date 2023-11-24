use std::{
    fmt::Debug,
    io,
    net::{SocketAddr, ToSocketAddrs},
};

use mio::Interest;

use hala_reactor::IoObject;

use super::TcpStream;

pub struct TcpListener {
    io: IoObject<mio::net::TcpListener>,
}

impl Debug for TcpListener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TcpListener(Token = {:?})", self.io.token)
    }
}

impl TcpListener {
    /// Create new tcp listener with calling underly bind method.
    pub fn bind<S: ToSocketAddrs>(laddr: S) -> io::Result<Self> {
        let std_listener = std::net::TcpListener::bind(laddr)?;

        std_listener.set_nonblocking(true)?;

        let io_device = hala_reactor::global_io_device().clone();

        let listener = mio::net::TcpListener::from_std(std_listener);

        Ok(Self {
            io: IoObject::new(io_device, listener),
        })
    }

    /// Accepts a new incoming connection from this listener.
    pub async fn accept(&self) -> io::Result<(TcpStream, SocketAddr)> {
        let (stream, addr) = self
            .io
            .async_io(Interest::READABLE, || self.io.inner_object.get().accept())
            .await?;

        Ok((TcpStream::from_mio(stream), addr))
    }

    /// Returns the local socket address of this listener.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.io.inner_object.get().local_addr()
    }
}
