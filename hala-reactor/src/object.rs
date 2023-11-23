use std::{future::Future, io, task::Poll};

use mio::{event, Interest, Token};

use crate::{IoDevice, ThreadModel};

#[derive(Debug)]
pub struct IoObject<T> {
    token: Token,
    pub inner_object: ThreadModel<T>,
    io_device: IoDevice,
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

    pub fn poll_io<R, F>(&self, interest: Interest, f: F) -> PollIo<'_, T, F>
    where
        F: Fn() -> io::Result<R>,
    {
        PollIo {
            f,
            interest,
            object: self,
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
    F: Fn() -> io::Result<R> + Unpin,
    T: event::Source,
{
    type Output = io::Result<R>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        loop {
            match (self.f)() {
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    let io_dervice = self.object.io_device.clone();
                    let token = self.object.token;
                    let interest = self.interest;

                    if let Err(err) = io_dervice.poll_register(
                        cx,
                        &mut *self.object.inner_object.get_mut(),
                        token,
                        interest,
                    ) {
                        return Poll::Ready(Err(err));
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::Interrupted => {
                    continue;
                }
                output => return Poll::Ready(output),
            }
        }
    }
}
