use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{AsyncRead, AsyncWrite, Future};

/// Hala io multiplexer trait.
pub trait Mux {
    /// This mux associated `Read` type.
    type Read: AsyncRead;
    /// /// This mux associated `Write` type.
    type Write: AsyncWrite;

    /// Open new stream object
    fn poll_open(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<(Self::Read, Self::Write)>>;

    /// Accept a new incoming stream
    fn poll_accept(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<(Self::Read, Self::Write)>>;
}

/// An extension trait for [`Mux`]
pub trait MuxExt: Mux {
    fn open(&mut self) -> Open<'_, Self>
    where
        Self: Sized + Unpin,
    {
        Open::new(self)
    }

    fn accept(&mut self) -> Accept<'_, Self>
    where
        Self: Sized + Unpin,
    {
        Accept::new(self)
    }
}

impl<M: Mux + ?Sized> MuxExt for M {}

/// Future for the [`open`](MuxExt::open) method.
pub struct Open<'a, M> {
    mux: &'a mut M,
}

impl<'a, M> Open<'a, M> {
    fn new(mux: &'a mut M) -> Self {
        Self { mux }
    }
}

impl<'a, M: Mux + Unpin> Future for Open<'a, M> {
    type Output = io::Result<(M::Read, M::Write)>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut *self.mux).poll_open(cx)
    }
}

/// Future for the [`accept`](MuxExt::accept) method.
pub struct Accept<'a, M> {
    mux: &'a mut M,
}

impl<'a, M> Accept<'a, M> {
    fn new(mux: &'a mut M) -> Self {
        Self { mux }
    }
}

impl<'a, M: Mux + Unpin> Future for Accept<'a, M> {
    type Output = io::Result<(M::Read, M::Write)>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut *self.mux).poll_accept(cx)
    }
}

/// Add [`mux`](IOMuxExt::mux) method to [`AsyncRead`] + [`AsyncWrite`] object
pub trait IOMuxExt: AsyncRead + AsyncWrite {
    /// Helper method to convert [`AsyncRead`] + [`AsyncWrite`] object `into` [`IOMux`] object
    fn mux<M: IOMux<IO = Self> + Mux>(self) -> M
    where
        Self: Sized,
    {
        IOMux::new(self)
    }
}

/// The [`Mux`] constructor from [`AsyncRead`] + [`AsyncWrite`] object
pub trait IOMux {
    type IO: AsyncRead + AsyncWrite;
    fn new(io: Self::IO) -> Self;
}
