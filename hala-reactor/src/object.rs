use std::{
    fmt::Debug,
    io,
    marker::PhantomData,
    task::{Context, Poll},
};

use mio::{event::Source, Interest, Token};

use crate::{AsyncIo, ContextIoDevice, IoDevice, IoDeviceExt, ThreadModelGuard};

/// The [`Source`] wrapper object.
#[derive(Clone)]
pub struct IoObject<IO: IoDevice + ContextIoDevice, S: Source> {
    pub token: Token,
    pub holder: IO::Guard<S>,
    io: IO,
    _marked: PhantomData<IO>,
}

impl<IO, S> IoObject<IO, S>
where
    IO: IoDevice + ContextIoDevice,
    S: Source,
{
    /// Create new [`IoObject`] by providing `S`
    pub fn new(mut inner: S, interests: Interest) -> io::Result<Self> {
        let io = IO::get();

        let token = io.register(&mut inner, interests)?;

        Ok(Self {
            io,
            token,
            holder: IO::Guard::new(inner),
            _marked: Default::default(),
        })
    }

    /// Call an nonblocking io operation and register async `waker` if the result is WOULD_BLOCk.
    #[inline]
    pub fn poll_io_with_context<R, F>(
        &self,
        cx: &mut Context<'_>,
        interests: Interest,
        f: F,
    ) -> Poll<io::Result<R>>
    where
        F: FnMut() -> io::Result<R>,
        R: Debug,
    {
        self.io.poll_io_with_context(cx, self.token, interests, f)
    }

    /// Call an nonblocking io operation
    #[inline]
    pub fn poll_io<R, F>(&self, _interests: Interest, f: F) -> Poll<io::Result<R>>
    where
        F: FnMut() -> io::Result<R>,
        R: Debug,
    {
        self.io.poll_io(f)
    }

    #[inline]
    pub fn async_io<F>(&self, interests: Interest, f: F) -> AsyncIo<'_, IO, F>
    where
        Self: Sized,
    {
        self.io.async_io(self.token, interests, f)
    }
}

impl<IO: IoDevice + ContextIoDevice, S: Source> Drop for IoObject<IO, S>
where
    S: Source,
{
    fn drop(&mut self) {
        self.io
            .deregister(&mut *self.holder.get_mut(), self.token)
            .unwrap()
    }
}
