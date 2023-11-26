use std::{
    io,
    marker::PhantomData,
    task::{Context, Poll},
};

use mio::{event::Source, Interest, Token};

use crate::{AsyncIo, ContextIoDevice, IoDevice, IoDeviceExt, ThreadModelGuard};

/// The [`Source`] wrapper object.
#[derive(Clone)]
pub struct IoObject<IO: IoDevice + ContextIoDevice + 'static, S: Source> {
    pub token: Token,
    pub holder: IO::Guard<S>,
    _marked: PhantomData<IO>,
}

impl<IO, S> IoObject<IO, S>
where
    IO: IoDevice + ContextIoDevice + 'static,
    S: Source,
{
    /// Create new [`IoObject`] by providing `S`
    pub fn new(mut inner: S, interests: Interest) -> io::Result<Self> {
        let token = IO::get().register(&mut inner, interests)?;

        Ok(Self {
            token,
            holder: IO::Guard::new(inner),
            _marked: Default::default(),
        })
    }

    /// invoke one poll io
    #[inline]
    pub fn poll_io<R, F>(
        &self,
        device: &IO,
        cx: &mut Context<'_>,
        interests: Interest,
        f: F,
    ) -> Poll<io::Result<R>>
    where
        F: FnMut() -> io::Result<R>,
    {
        device.poll_io(cx, self.token, interests, f)
    }

    #[inline]
    pub fn async_io<'a, F>(&self, device: &'a IO, interests: Interest, f: F) -> AsyncIo<'a, IO, F>
    where
        Self: Sized,
    {
        device.async_io(self.token, interests, f)
    }
}

impl<IO: IoDevice + ContextIoDevice + 'static, S: Source> Drop for IoObject<IO, S>
where
    S: Source,
{
    fn drop(&mut self) {
        IO::get()
            .deregister(&mut *self.holder.get_mut(), self.token)
            .unwrap()
    }
}
