use std::{
    fmt::Debug,
    io,
    net::{SocketAddr, ToSocketAddrs},
};

use mio::Interest;

use hala_reactor::{ContextIoDevice, IoDevice, IoObject, MioDevice, ThreadModelGuard};

use super::TcpStream;

pub struct TcpListener<IO: IoDevice + ContextIoDevice + 'static = MioDevice> {
    io: IoObject<IO, mio::net::TcpListener>,
}

impl<IO: IoDevice + ContextIoDevice> Debug for TcpListener<IO> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TcpListener(Token = {:?})", self.io.token)
    }
}

impl<IO: IoDevice + ContextIoDevice> TcpListener<IO> {
    /// Create new tcp listener with calling underly bind method.
    pub fn bind<S: ToSocketAddrs>(laddr: S) -> io::Result<Self> {
        let std_listener = std::net::TcpListener::bind(laddr)?;

        std_listener.set_nonblocking(true)?;

        let listener = mio::net::TcpListener::from_std(std_listener);

        Ok(Self {
            io: IoObject::new(listener, Interest::READABLE)?,
        })
    }

    /// Accepts a new incoming connection from this listener.
    pub async fn accept(&self) -> io::Result<(TcpStream, SocketAddr)> {
        let io = IO::get();

        let (stream, addr) = self
            .io
            .async_io(&io, Interest::READABLE, || self.io.holder.get().accept())
            .await?;

        Ok((TcpStream::from_mio(stream)?, addr))
    }

    /// Returns the local socket address of this listener.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.io.holder.get().local_addr()
    }
}
