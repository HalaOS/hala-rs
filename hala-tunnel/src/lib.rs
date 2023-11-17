#![no_std]
pub extern crate alloc;

use core::future::Future;

use core::pin::Pin;
use core::{
    fmt::Debug,
    task::{Context, Poll},
};

/// Hala tunnel error varaint
#[derive(Debug, thiserror_no_std::Error)]
pub enum HalaTunnelError {}

/// [`Result`] type for [`HalaTunnel`] APIs.
pub type HalaTunnelResult<T> = Result<T, HalaTunnelError>;

pub type HalaTunnelStreamID = usize;

/// Opened stream object of [`HalaTunnel`]
pub trait HalaTunnelStream: Debug + Sync + Send {
    /// Attempts to write data to the stream.
    fn poll_write(&self, cx: &mut Context<'_>, buf: &[u8]) -> Poll<core2::io::Result<usize>>;

    /// Attempt to read from the stream into buf.
    fn poll_read(&self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<core2::io::Result<usize>>;

    /// Close stream object
    fn poll_close(&self, cx: &mut Context<'_>) -> Poll<core2::io::Result<()>>;
}

/// An extension trait which adds utility methods to [`HalaTunnelStream`] types.
pub trait HalaTunnelStreamEx: HalaTunnelStream {
    /// Creates a future which will write bytes from `buf` into the object.
    fn write<'a>(&'a self, buf: &'a [u8]) -> Write<'_, Self>
    where
        Self: Unpin,
    {
        Write::new(self, buf)
    }

    /// Creates a future which will read bytes from object into the `buf`.
    fn read<'a>(&'a self, buf: &'a mut [u8]) -> Read<'_, Self>
    where
        Self: Unpin,
    {
        Read::new(self, buf)
    }

    /// Creates a future which will entirely close this [`HalaTunnelStream`].
    fn close(&self) -> Close<'_, Self>
    where
        Self: Unpin,
    {
        Close::new(self)
    }
}

impl<P: HalaTunnelStream + ?Sized> HalaTunnelStreamEx for P {}

/// Future for the [`write`](HalaTunnelStreamEx::write) method.
pub struct Write<'a, S: ?Sized> {
    stream: &'a S,
    buf: &'a [u8],
}

impl<S: ?Sized + Unpin> Unpin for Write<'_, S> {}

impl<'a, S: ?Sized> Write<'a, S> {
    fn new(stream: &'a S, buf: &'a [u8]) -> Self {
        Self { stream, buf }
    }
}

impl<'a, S: ?Sized + Unpin + HalaTunnelStream> Future for Write<'a, S> {
    type Output = core2::io::Result<usize>;
    fn poll(self: core::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&*self.stream).poll_write(cx, self.buf)
    }
}

/// Future for the [`read`](HalaTunnelStreamEx::read) method.
pub struct Read<'a, S: ?Sized> {
    stream: &'a S,
    buf: &'a mut [u8],
}

impl<S: ?Sized + Unpin> Unpin for Read<'_, S> {}

impl<'a, S: ?Sized> Read<'a, S> {
    fn new(stream: &'a S, buf: &'a mut [u8]) -> Self {
        Self { stream, buf }
    }
}

impl<'a, S: ?Sized + Unpin + HalaTunnelStream> Future for Read<'a, S> {
    type Output = core2::io::Result<usize>;
    fn poll(mut self: core::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&*self.stream).poll_read(cx, self.buf)
    }
}

/// Future for the [`close`](HalaTunnelStreamEx::close) method.
pub struct Close<'a, S: ?Sized> {
    stream: &'a S,
}

impl<S: ?Sized + Unpin> Unpin for Close<'_, S> {}

impl<'a, S: ?Sized> Close<'a, S> {
    fn new(stream: &'a S) -> Self {
        Self { stream }
    }
}

impl<'a, S: ?Sized + Unpin + HalaTunnelStream> Future for Close<'a, S> {
    type Output = core2::io::Result<()>;
    fn poll(self: core::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&*self.stream).poll_close(cx)
    }
}

/// Hala tunnel  implementation.
pub trait HalaTunnel: Debug + Sync + Send {
    /// Associated stream  object type
    type Stream: HalaTunnelStream;

    /// Create new out stream
    fn poll_open_stream(&self, cx: &mut Context<'_>) -> Poll<core2::io::Result<Self::Stream>>;

    /// Accept one incoming stream.
    fn poll_accept(&self, cx: &mut Context<'_>) -> Poll<core2::io::Result<Self::Stream>>;
}

/// An extension trait which adds utility methods to [`HalaTunnel`] types.
pub trait HalaTunnelEx: HalaTunnel {
    fn open_stream(&self) -> OpenStream<'_, Self>
    where
        Self: Unpin,
    {
        OpenStream::new(self)
    }

    fn accept(&self) -> Accept<'_, Self>
    where
        Self: Unpin,
    {
        Accept::new(self)
    }
}

/// Future for the [`open_stream`](HalaTunnelEx::accept) method.
pub struct Accept<'a, T: ?Sized> {
    tunnel: &'a T,
}

impl<T: ?Sized + Unpin> Unpin for Accept<'_, T> {}

impl<'a, T: ?Sized> Accept<'a, T> {
    fn new(tunnel: &'a T) -> Self {
        Self { tunnel }
    }
}

impl<'a, T: ?Sized + Unpin + HalaTunnel> Future for Accept<'a, T> {
    type Output = core2::io::Result<T::Stream>;
    fn poll(self: core::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&*self.tunnel).poll_accept(cx)
    }
}

/// Future for the [`open_stream`](HalaTunnelEx::open_stream) method.
pub struct OpenStream<'a, T: ?Sized> {
    tunnel: &'a T,
}

impl<T: ?Sized + Unpin> Unpin for OpenStream<'_, T> {}

impl<'a, T: ?Sized> OpenStream<'a, T> {
    fn new(tunnel: &'a T) -> Self {
        Self { tunnel }
    }
}

impl<'a, T: ?Sized + Unpin + HalaTunnel> Future for OpenStream<'a, T> {
    type Output = core2::io::Result<T::Stream>;
    fn poll(self: core::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&*self.tunnel).poll_open_stream(cx)
    }
}

impl<P: HalaTunnel + ?Sized> HalaTunnelEx for P {}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct MockHalaTunnel {}

    impl HalaTunnel for MockHalaTunnel {
        type Stream = MockHalaTunnelStream;

        fn poll_open_stream(&self, _cx: &mut Context<'_>) -> Poll<core2::io::Result<Self::Stream>> {
            Poll::Ready(Ok(MockHalaTunnelStream {}))
        }

        fn poll_accept(&self, _cx: &mut Context<'_>) -> Poll<core2::io::Result<Self::Stream>> {
            Poll::Ready(Ok(MockHalaTunnelStream {}))
        }
    }

    #[derive(Debug)]
    struct MockHalaTunnelStream {}

    impl HalaTunnelStream for MockHalaTunnelStream {
        fn poll_write(&self, _cx: &mut Context<'_>, buf: &[u8]) -> Poll<core2::io::Result<usize>> {
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_read(
            &self,
            _cx: &mut Context<'_>,
            buf: &mut [u8],
        ) -> Poll<core2::io::Result<usize>> {
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_close(&self, _cx: &mut Context<'_>) -> Poll<core2::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[futures_test::test]
    async fn test_ext_fns() {
        let tunnel = MockHalaTunnel {};

        tunnel.open_stream().await.unwrap();

        let stream = tunnel.accept().await.unwrap();

        stream.write(b"hello").await.unwrap();

        let mut buf = [0 as u8; 32];

        stream.read(&mut buf).await.unwrap();
    }
}
