use std::{
    io,
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use mio::{event::Source, Interest, Token};

use crate::IoDevice;

/// The [`Source`] wrapper object.
#[derive(Clone)]
pub struct IoObject<IO: IoDevice, S: Source> {
    token: Token,
    inner: S,
    _marked: PhantomData<IO>,
}

impl<IO: IoDevice, S: Source> IoObject<IO, S> {
    /// Create new [`IoObject`] by providing `S`
    pub fn new(mut inner: S, interests: Interest) -> io::Result<Self> {
        let token = IO::get().register(&mut inner, interests)?;

        Ok(Self {
            token,
            inner,
            _marked: Default::default(),
        })
    }
}

impl<IO: IoDevice, S: Source> Deref for IoObject<IO, S> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<IO: IoDevice, S: Source> DerefMut for IoObject<IO, S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<IO: IoDevice, S: Source> Drop for IoObject<IO, S> {
    fn drop(&mut self) {
        IO::get().deregister(&mut self.inner, self.token).unwrap();
    }
}
