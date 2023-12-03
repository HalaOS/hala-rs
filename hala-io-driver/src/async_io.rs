use std::{
    future::Future,
    io,
    task::{Context, Poll},
};

pub struct AsyncIo<F> {
    f: F,
}

impl<F, R> Future for AsyncIo<F>
where
    F: FnMut(&mut Context<'_>) -> io::Result<R> + Unpin,
{
    type Output = io::Result<R>;

    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match (self.f)(cx) {
            Ok(r) => Poll::Ready(Ok(r)),
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
            Err(err) => Poll::Ready(Err(err)),
        }
    }
}

/// Create new `Future` from nonbloking io operation.
pub fn async_io<F, R>(f: F) -> AsyncIo<F>
where
    F: FnMut(&mut Context<'_>) -> io::Result<R> + Unpin,
{
    AsyncIo { f }
}

pub fn poll<F, R>(f: F) -> Poll<io::Result<R>>
where
    F: FnOnce() -> io::Result<R> + Unpin,
{
    match f() {
        Ok(r) => Poll::Ready(Ok(r)),
        Err(err) if err.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
        Err(err) => Poll::Ready(Err(err)),
    }
}
