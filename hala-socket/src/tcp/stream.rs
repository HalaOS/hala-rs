use std::{
    fmt::Debug,
    io::{self, Read, Write},
    net::ToSocketAddrs,
    task::Poll,
};

use futures::{AsyncRead, AsyncWrite};
use hala_reactor::{global_io_device, IoObject};
use mio::Interest;

pub struct TcpStream {
    io: IoObject<mio::net::TcpStream>,
}

impl Debug for TcpStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TcpStream({:?})", self.io.token)
    }
}

impl TcpStream {
    /// Opens a TCP connection to a remote host.
    pub async fn connect<S: ToSocketAddrs>(addr: S) -> io::Result<Self> {
        let std_stream = std::net::TcpStream::connect(addr)?;

        std_stream.set_nonblocking(true)?;

        let io = IoObject::new(
            global_io_device().clone(),
            mio::net::TcpStream::from_std(std_stream),
        );

        Ok(Self { io })
    }

    /// Create object from [`mio::net::TcpStream`]
    pub(crate) fn from_mio(stream: mio::net::TcpStream) -> Self {
        let io = IoObject::new(global_io_device().clone(), stream);

        Self { io }
    }
}

impl AsyncWrite for TcpStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        self.io.poll_io(cx, Interest::WRITABLE, || {
            self.io.inner_object.get_mut().write(buf)
        })
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        Poll::Ready(self.io.inner_object.get_mut().flush())
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        Poll::Ready(
            self.io
                .inner_object
                .get_mut()
                .shutdown(std::net::Shutdown::Both),
        )
    }
}

impl AsyncRead for TcpStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        self.io.poll_io(cx, Interest::READABLE, || {
            self.io.inner_object.get_mut().read(buf)
        })
    }
}
