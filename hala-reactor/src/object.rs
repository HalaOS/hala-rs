use std::{fmt::Debug, future::Future, io, task::Poll};

use mio::{event, Interest, Token};

use crate::{IoDevice, ThreadModel};

pub trait ToIoObject<T>
where
    T: event::Source,
{
    fn to_io_object(&self) -> &IoObject<T>;
}

#[derive(Debug)]
pub struct IoObject<T>
where
    T: event::Source,
{
    pub token: Token,
    pub inner_object: ThreadModel<T>,
    pub io_device: IoDevice,
    pub interests: Interest,
}

impl<T> IoObject<T>
where
    T: event::Source,
{
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
    /// Create new io object wrapper
    pub fn new(io_device: IoDevice, mut inner_object: T, interests: Interest) -> io::Result<Self> {
        let token = io_device.new_token();

        io_device.source_register(&mut inner_object, token, interests)?;

        Ok(Self {
            token,
            inner_object: inner_object.into(),
            io_device,
            interests,
        })
    }

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

                    if let Err(err) = io_dervice.poll_register(cx, token, interest) {
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

impl<T> Drop for IoObject<T>
where
    T: event::Source,
{
    fn drop(&mut self) {
        self.io_device
            .source_deregister(&mut *self.inner_object.get_mut())
            .unwrap();
    }
}

pub struct PollIo<'a, T, F>
where
    T: event::Source,
{
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

                    if let Err(err) = io_dervice.poll_register(cx, token, interest) {
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

/// Select one ready [`IoObject`] and execute `poll_fn`.
pub fn select_io_object<'a, T, F>(
    io_objects: &'a [&'a T],
    poll_fn: F,
) -> IoObjectSelector<'a, T, F> {
    IoObjectSelector::new(io_objects, poll_fn)
}

pub struct IoObjectSelector<'a, T, F> {
    io_objects: &'a [&'a T],
    poll_fn: Option<F>,
}

impl<'a, T, F> IoObjectSelector<'a, T, F> {
    pub fn new(io_objects: &'a [&'a T], poll_fn: F) -> Self {
        Self {
            io_objects,
            poll_fn: Some(poll_fn),
        }
    }
}

impl<'a, T, F, R> Future for IoObjectSelector<'a, T, F>
where
    F: FnMut(&mut std::task::Context<'_>, &T) -> Poll<io::Result<R>> + Unpin,
{
    type Output = io::Result<R>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Self::Output> {
        let mut poll_fn = self.as_mut().poll_fn.take().unwrap();

        for object in self.io_objects {
            match poll_fn(cx, *object) {
                Poll::Pending => continue,
                ready => return ready,
            }
        }

        Poll::Pending
    }
}
