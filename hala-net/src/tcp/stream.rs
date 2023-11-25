use std::{
    fmt::Debug,
    io::{self, Read, Write},
    net::ToSocketAddrs,
    task::Poll,
};

use futures::{AsyncRead, AsyncWrite};
use hala_reactor::{IoDevice, IoObject, MioDevice, StaticIoDevice, ThreadModelHolder};
use mio::Interest;

pub struct TcpStream<IO: IoDevice + StaticIoDevice + 'static = MioDevice> {
    io: IoObject<IO, mio::net::TcpStream>,
}

impl<IO: IoDevice + StaticIoDevice + 'static> Debug for TcpStream<IO> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TcpStream({:?})", self.io.token)
    }
}

impl<IO: IoDevice + StaticIoDevice + 'static> TcpStream<IO> {
    /// Opens a TCP connection to a remote host.
    pub async fn connect<S: ToSocketAddrs>(addr: S) -> io::Result<Self> {
        let std_stream = std::net::TcpStream::connect(addr)?;

        std_stream.set_nonblocking(true)?;

        let io = IoObject::new(
            mio::net::TcpStream::from_std(std_stream),
            Interest::READABLE.add(Interest::WRITABLE),
        )?;

        Ok(Self { io })
    }

    /// Create object from [`mio::net::TcpStream`]
    pub(crate) fn from_mio(stream: mio::net::TcpStream) -> io::Result<Self> {
        let io = IoObject::new(stream, Interest::READABLE.add(Interest::WRITABLE))?;

        Ok(Self { io })
    }
}

impl<IO: IoDevice + StaticIoDevice + 'static> AsyncWrite for TcpStream<IO> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        self.io.poll_io(IO::get(), cx, Interest::WRITABLE, || {
            self.io.holder.get_mut().write(buf)
        })
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        Poll::Ready(self.io.holder.get_mut().flush())
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        Poll::Ready(self.io.holder.get().shutdown(std::net::Shutdown::Both))
    }
}

impl<IO: IoDevice + StaticIoDevice + 'static> AsyncRead for TcpStream<IO> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        self.io.poll_io(IO::get(), cx, Interest::READABLE, || {
            self.io.holder.get_mut().read(buf)
        })
    }
}
