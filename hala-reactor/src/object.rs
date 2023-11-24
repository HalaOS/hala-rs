use std::{fmt::Debug, future::Future, io, task::Poll};

use mio::{event, Interest, Token};

use crate::{IoDevice, ThreadModel};

#[derive(Debug)]
pub struct IoObject<T> {
    pub token: Token,
    pub inner_object: ThreadModel<T>,
    pub io_device: IoDevice,
}

impl<T> IoObject<T> {
    /// Create new io object wrapper
    pub fn new(io_device: IoDevice, inner_object: T) -> Self {
        Self {
            token: io_device.new_token(),
            inner_object: inner_object.into(),
            io_device,
        }
    }

    pub fn async_io<R, F>(&self, interest: Interest, f: F) -> PollIo<'_, T, F>
    where
        F: FnMut() -> io::Result<R>,
    {
        PollIo {
            f,
            interest,
            object: self,
        }
    }
}

impl<T> IoObject<T>
where
    T: event::Source,
{
    /// Invoke one io ops.
    pub fn poll_io<R, F>(
        &self,
        cx: &mut std::task::Context<'_>,
        interest: Interest,
        mut f: F,
    ) -> Poll<io::Result<R>>
    where
        F: FnMut() -> io::Result<R>,
        R: Debug,
    {
        loop {
            match f() {
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    let io_dervice = self.io_device.clone();
                    let token = self.token;
                    let interest = interest;

                    log::trace!("IoObject({:?}) ops {:?} WouldBlock", self.token, interest);

                    if let Err(err) = io_dervice.poll_register(
                        cx,
                        &mut *self.inner_object.get_mut(),
                        token,
                        interest,
                    ) {
                        return Poll::Ready(Err(err));
                    }

                    return Poll::Pending;
                }
                Err(err) if err.kind() == io::ErrorKind::Interrupted => {
                    log::trace!("IoObject({:?}) Interrupted", self.token);
                    continue;
                }
                output => {
                    log::trace!(
                        "IoObject({:?}) complete ops {:?},{:?}",
                        self.token,
                        interest,
                        output
                    );

                    return Poll::Ready(output);
                }
            }
        }
    }
}

pub struct PollIo<'a, T, F> {
    f: F,
    interest: Interest,
    object: &'a IoObject<T>,
}

impl<'a, R, T, F> Future for PollIo<'a, T, F>
where
    F: FnMut() -> io::Result<R> + Unpin,
    T: event::Source,
    R: Debug,
{
    type Output = io::Result<R>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        loop {
            match (self.f)() {
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    let io_dervice = self.object.io_device.clone();
                    let token = self.object.token;
                    let interest = self.interest;

                    log::trace!(
                        "IoObject({:?}) ops {:?} WouldBlock",
                        self.object.token,
                        self.interest
                    );

                    if let Err(err) = io_dervice.poll_register(
                        cx,
                        &mut *self.object.inner_object.get_mut(),
                        token,
                        interest,
                    ) {
                        return Poll::Ready(Err(err));
                    }

                    return Poll::Pending;
                }
                Err(err) if err.kind() == io::ErrorKind::Interrupted => {
                    log::trace!("IoObject({:?}) Interrupted", self.object.token);
                    continue;
                }
                output => {
                    log::trace!(
                        "IoObject({:?}) complete ops {:?},{:?}",
                        self.object.token,
                        self.interest,
                        output
                    );

                    return Poll::Ready(output);
                }
            }
        }
    }
}
