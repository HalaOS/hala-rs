use std::{
    collections::HashMap,
    io,
    sync::{atomic::AtomicUsize, Arc},
    task::{Context, Poll, Waker},
};

use futures::Future;
use mio::{event::Source, Interest, Token};

use crate::{MTModel, ThreadModel, ThreadModelHolder};

/// Poll reactor device must implement this trait
pub trait IoDevice {
    type Holder<T>: ThreadModelHolder<T>;

    /// Get `device` instance.
    fn get() -> &'static Self;

    /// Re-register an [`Source`] with the reactor device.
    fn register<S>(&self, source: &mut S, interests: Interest) -> io::Result<Token>
    where
        S: Source;

    /// Re-register an [`Source`] with the reactor device.
    fn reregister<S>(
        &self,
        source: &mut S,
        old_token: Token,
        interests: Interest,
    ) -> io::Result<Token>
    where
        S: Source;

    /// Deregister an [`Source`] with the reactor device.
    fn deregister<S>(&self, source: &mut S, token: Token) -> io::Result<()>
    where
        S: Source;

    /// invoke one poll io
    fn poll_io<R, F>(
        &self,
        cx: &mut Context<'_>,
        token: Token,
        interests: Interest,
        f: F,
    ) -> Poll<io::Result<R>>
    where
        F: FnMut() -> io::Result<R>;
}

/// Extension trait for [`IoDevice`]
pub trait IoDeviceExt: IoDevice {
    /// Create a new asynchronous io calling with nonblock sync `F`
    fn async_io<F>(&self, token: Token, interests: Interest, f: F) -> AsyncIo<'_, Self, F>
    where
        Self: Sized,
    {
        AsyncIo {
            io: self,
            token,
            interests,
            f,
        }
    }

    /// Select one ready io object and execute method `f`
    fn select<'a, F>(
        &'a self,
        tokens: &'a [Token],
        interests: Interest,
        f: F,
    ) -> SelectIo<'_, Self, F>
    where
        Self: Sized,
    {
        SelectIo {
            io: self,
            tokens,
            interests,
            f,
        }
    }
}

impl<T: IoDevice> IoDeviceExt for T {}

/// Future object returns by [`IoDeviceExt::async_io`]
pub struct AsyncIo<'a, IO, F> {
    io: &'a IO,
    token: Token,
    interests: Interest,
    f: F,
}

impl<'a, IO, F, R> Future for AsyncIo<'a, IO, F>
where
    IO: IoDevice,
    F: FnMut() -> io::Result<R> + Unpin,
{
    type Output = io::Result<R>;

    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.io.poll_io(cx, self.token, self.interests, &mut self.f)
    }
}

/// Future object returns by [`IoDeviceExt::async_io`]
#[allow(unused)]
pub struct SelectIo<'a, IO, F> {
    io: &'a IO,
    tokens: &'a [Token],
    interests: Interest,
    f: F,
}

impl<'a, IO, F, R> Future for SelectIo<'a, IO, F>
where
    IO: IoDevice,
    F: FnMut(&mut Context<'_>) -> io::Result<R> + Unpin,
{
    type Output = io::Result<R>;

    fn poll(self: std::pin::Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        todo!()
    }
}

/// IoDevice using mio implementation
#[derive(Clone)]
pub struct MioDevice<TM: ThreadModel = MTModel> {
    poll: TM::Holder<mio::Poll>,
    /// readable waker collection
    read_wakers: TM::Holder<HashMap<Token, Waker>>,
    /// writable waker collection
    write_wakers: TM::Holder<HashMap<Token, Waker>>,
    /// token generating seed
    next_token: Arc<AtomicUsize>,
}

impl<TM: ThreadModel> MioDevice<TM> {
    /// Create new io device object.
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            read_wakers: HashMap::default().into(),
            write_wakers: HashMap::default().into(),
            poll: mio::Poll::new()?.into(),
            next_token: Default::default(),
        })
    }

    /// Create new [`mio::Token`](mio::Token), which will generate seqenuce increasing token number
    pub fn new_token(&self) -> Token {
        use std::sync::atomic::Ordering;

        let next_token = self.next_token.fetch_add(1, Ordering::SeqCst);

        let token = Token(next_token);

        log::trace!("next token {:?}", token);

        token
    }
}

impl<TM: ThreadModel> IoDevice for MioDevice<TM> {
    type Holder<T> = TM::Holder<T>;

    fn get() -> &'static Self {
        todo!()
    }

    fn register<S>(&self, source: &mut S, interests: Interest) -> io::Result<Token>
    where
        S: Source,
    {
        let token = self.new_token();

        self.poll
            .get()
            .registry()
            .register(source, token, interests)?;

        Ok(token)
    }

    fn reregister<S>(
        &self,
        source: &mut S,
        old_token: Token,
        interests: Interest,
    ) -> io::Result<Token>
    where
        S: Source,
    {
        let token = self.new_token();

        self.poll
            .get()
            .registry()
            .reregister(source, token, interests)?;

        self.read_wakers.get_mut().remove(&old_token);
        self.write_wakers.get_mut().remove(&old_token);

        Ok(token)
    }

    fn deregister<S>(&self, source: &mut S, token: Token) -> io::Result<()>
    where
        S: Source,
    {
        self.poll.get().registry().deregister(source)?;

        self.read_wakers.get_mut().remove(&token);
        self.write_wakers.get_mut().remove(&token);

        Ok(())
    }

    fn poll_io<R, F>(
        &self,
        cx: &mut Context<'_>,
        token: Token,
        interests: Interest,
        mut f: F,
    ) -> Poll<io::Result<R>>
    where
        F: FnMut() -> io::Result<R>,
    {
        loop {
            match f() {
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    if interests.is_readable() {
                        self.read_wakers.get_mut().insert(token, cx.waker().clone());
                    } else if interests.is_writable() {
                        self.write_wakers
                            .get_mut()
                            .insert(token, cx.waker().clone());
                    }
                    return Poll::Pending;
                }
                Err(err) if err.kind() == io::ErrorKind::Interrupted => {
                    log::trace!("IoObject({:?}) Interrupted", token);
                    continue;
                }
                output => return Poll::Ready(output),
            }
        }
    }
}
