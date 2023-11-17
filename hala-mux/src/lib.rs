#![no_std]
pub extern crate alloc;

use core::future::Future;

use core::pin::Pin;
use core::task::{Context, Poll};

use futures::{AsyncRead, AsyncWrite};

/// Opened stream object of [`HalaMux`]
pub trait HalaMuxStream {
    /// Attempts to write data to the stream.
    fn poll_write<W: AsyncWrite + Unpin>(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        writer: &mut W,
        buf: &[u8],
    ) -> Poll<core2::io::Result<usize>>;

    /// Attempt to read from the stream into buf.
    fn poll_read<R: AsyncRead + Unpin>(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        reader: &mut R,
        buf: &mut [u8],
    ) -> Poll<core2::io::Result<usize>>;

    /// Close stream object
    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<core2::io::Result<()>>;
}

/// An extension trait which adds utility methods to [`HalaMuxStream`] types.
pub trait HalaMuxStreamExt: HalaMuxStream {
    /// Creates a future which will write bytes from `buf` into the object.
    fn write<'a, W: AsyncWrite + Unpin>(
        &'a mut self,
        writer: &'a mut W,
        buf: &'a [u8],
    ) -> Write<'_, Self, W>
    where
        Self: Unpin,
    {
        Write::new(self, writer, buf)
    }

    /// Creates a future which will read bytes from object into the `buf`.
    fn read<'a, R: AsyncRead + Unpin>(
        &'a mut self,
        reader: &'a mut R,
        buf: &'a mut [u8],
    ) -> Read<'_, Self, R>
    where
        Self: Unpin,
    {
        Read::new(self, reader, buf)
    }

    /// Creates a future which will entirely close this [`HalaMuxStream`].
    fn close(&mut self) -> Close<'_, Self>
    where
        Self: Unpin,
    {
        Close::new(self)
    }
}

impl<P: HalaMuxStream + ?Sized> HalaMuxStreamExt for P {}

/// Future for the [`write`](HalaMuxStreamExt::write) method.
pub struct Write<'a, S: ?Sized, W> {
    stream: &'a mut S,
    buf: &'a [u8],
    writer: &'a mut W,
}

impl<S: ?Sized + Unpin, W> Unpin for Write<'_, S, W> {}

impl<'a, S: ?Sized, W> Write<'a, S, W> {
    fn new(stream: &'a mut S, writer: &'a mut W, buf: &'a [u8]) -> Self {
        Self {
            stream,
            buf,
            writer,
        }
    }
}

impl<'a, S: ?Sized + Unpin + HalaMuxStream, W: AsyncWrite + Unpin> Future for Write<'a, S, W> {
    type Output = core2::io::Result<usize>;
    fn poll(mut self: core::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;

        Pin::new(&mut *this.stream).poll_write(cx, this.writer, this.buf)
    }
}

/// Future for the [`read`](HalaMuxStreamExt::read) method.
pub struct Read<'a, S: ?Sized, R> {
    stream: &'a mut S,
    buf: &'a mut [u8],
    reader: &'a mut R,
}

impl<S: ?Sized + Unpin, R> Unpin for Read<'_, S, R> {}

impl<'a, S: ?Sized, R> Read<'a, S, R> {
    fn new(stream: &'a mut S, reader: &'a mut R, buf: &'a mut [u8]) -> Self {
        Self {
            stream,
            buf,
            reader,
        }
    }
}

impl<'a, S: ?Sized + Unpin + HalaMuxStream, R: AsyncRead + Unpin> Future for Read<'a, S, R> {
    type Output = core2::io::Result<usize>;
    fn poll(mut self: core::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;

        Pin::new(&mut *this.stream).poll_read(cx, this.reader, this.buf)
    }
}

/// Future for the [`close`](HalaMuxStreamExt::close) method.
pub struct Close<'a, S: ?Sized> {
    stream: &'a mut S,
}

impl<S: ?Sized + Unpin> Unpin for Close<'_, S> {}

impl<'a, S: ?Sized> Close<'a, S> {
    fn new(stream: &'a mut S) -> Self {
        Self { stream }
    }
}

impl<'a, S: ?Sized + Unpin + HalaMuxStream> Future for Close<'a, S> {
    type Output = core2::io::Result<()>;
    fn poll(mut self: core::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut *self.stream).poll_close(cx)
    }
}

/// Hala tunnel  implementation.
pub trait HalaMux {
    /// Associated stream  object type
    type Stream: HalaMuxStream;

    /// Create new out stream
    fn poll_open_stream(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<core2::io::Result<Self::Stream>>;

    /// Accept one incoming stream.
    fn poll_accept_stream(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<core2::io::Result<Self::Stream>>;
}

/// An extension trait which adds utility methods to [`HalaMux`] types.
pub trait HalaMuxExt: HalaMux {
    fn open_stream(&mut self) -> OpenStream<'_, Self>
    where
        Self: Unpin,
    {
        OpenStream::new(self)
    }

    fn accept_stream(&mut self) -> Accept<'_, Self>
    where
        Self: Unpin,
    {
        Accept::new(self)
    }
}

/// Future for the [`open_stream`](HalaMuxExt::accept) method.
pub struct Accept<'a, T: ?Sized> {
    tunnel: &'a mut T,
}

impl<T: ?Sized + Unpin> Unpin for Accept<'_, T> {}

impl<'a, T: ?Sized> Accept<'a, T> {
    fn new(tunnel: &'a mut T) -> Self {
        Self { tunnel }
    }
}

impl<'a, T: ?Sized + Unpin + HalaMux> Future for Accept<'a, T> {
    type Output = core2::io::Result<T::Stream>;
    fn poll(mut self: core::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut *self.tunnel).poll_accept_stream(cx)
    }
}

/// Future for the [`open_stream`](HalaMuxExt::open_stream) method.
pub struct OpenStream<'a, T: ?Sized> {
    tunnel: &'a mut T,
}

impl<T: ?Sized + Unpin> Unpin for OpenStream<'_, T> {}

impl<'a, T: ?Sized> OpenStream<'a, T> {
    fn new(tunnel: &'a mut T) -> Self {
        Self { tunnel }
    }
}

impl<'a, T: ?Sized + Unpin + HalaMux> Future for OpenStream<'a, T> {
    type Output = core2::io::Result<T::Stream>;
    fn poll(mut self: core::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut *self.tunnel).poll_open_stream(cx)
    }
}

impl<P: HalaMux + ?Sized> HalaMuxExt for P {}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use futures::io::Cursor;

    use super::*;

    #[derive(Debug)]
    struct MockHalaMux {}

    impl HalaMux for MockHalaMux {
        type Stream = MockHalaMuxStream;

        fn poll_open_stream(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<core2::io::Result<Self::Stream>> {
            Poll::Ready(Ok(MockHalaMuxStream {}))
        }

        fn poll_accept_stream(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<core2::io::Result<Self::Stream>> {
            Poll::Ready(Ok(MockHalaMuxStream {}))
        }
    }

    #[derive(Debug)]
    struct MockHalaMuxStream {}

    impl HalaMuxStream for MockHalaMuxStream {
        fn poll_write<W: AsyncWrite + Unpin>(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            writer: &mut W,
            buf: &[u8],
        ) -> Poll<core2::io::Result<usize>> {
            Pin::new(writer).poll_write(cx, buf)
        }

        fn poll_read<R: AsyncRead + Unpin>(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            reader: &mut R,
            buf: &mut [u8],
        ) -> Poll<core2::io::Result<usize>> {
            Pin::new(reader).poll_read(cx, buf)
        }

        fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<core2::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[futures_test::test]
    async fn test_ext_fns() {
        let mut reader = Cursor::new([1, 2, 3, 4]);
        let mut writer = Cursor::new(vec![0u8; 5]);

        let mut tunnel = MockHalaMux {};

        tunnel.open_stream().await.unwrap();

        let mut stream = tunnel.accept_stream().await.unwrap();

        stream.write(&mut writer, b"hello").await.unwrap();

        let mut buf = [0 as u8; 32];

        let len = stream.read(&mut reader, &mut buf).await.unwrap();

        assert_eq!(len, 4);
    }
}
