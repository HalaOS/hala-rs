use std::{future::Future, io, task::Context};

use std::task::Poll;

/// A future object which will suspend current task when `F` returns error [`WouldBlock`](io::ErrorKind::WouldBlock)
pub struct WouldBlock<F> {
    f: F,
}

impl<F, R> Future for WouldBlock<F>
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

/// Create a new future task that will suspend current task when `F` return error [`WouldBlock`](io::ErrorKind::WouldBlock)
pub fn would_block<F, R>(f: F) -> WouldBlock<F>
where
    F: FnMut(&mut Context<'_>) -> io::Result<R> + Unpin,
{
    WouldBlock { f }
}

/// Call function `F` once and convert returns error [`WouldBlock`](io::ErrorKind::WouldBlock) to Poll [`Pending`](Poll::Pending)
pub fn poll_would_block<F, R>(f: F) -> Poll<io::Result<R>>
where
    F: FnOnce() -> io::Result<R> + Unpin,
{
    match f() {
        Ok(r) => Poll::Ready(Ok(r)),
        Err(err) if err.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
        Err(err) => Poll::Ready(Err(err)),
    }
}
