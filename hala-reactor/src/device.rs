use std::{
    collections::HashMap,
    fmt::Debug,
    io,
    sync::{atomic::AtomicUsize, Arc, OnceLock},
    task::{Context, Poll, Waker},
    time::Duration,
};

use futures::{future::BoxFuture, Future};
use mio::{
    event::{Event, Source},
    Events, Interest, Token,
};

use crate::{MTModel, STModel, ThreadModel, ThreadModelGuard};

/// Poll reactor device must implement this trait
pub trait IoDevice {
    type Guard<T>: ThreadModelGuard<T>;

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

    fn register_token_waker(
        &self,
        cx: &mut Context<'_>,
        token: &Token,
        interests: Interest,
    ) -> io::Result<()>;

    fn deregister_token_waker(&self, token: Token, interests: Interest) -> io::Result<()>;

    /// invoke one poll io, and register waker if `WOULD_BLOCK`
    fn poll_io_with_context<R, F>(
        &self,
        cx: &mut Context<'_>,
        token: Token,
        interests: Interest,
        f: F,
    ) -> Poll<io::Result<R>>
    where
        F: FnMut() -> io::Result<R>,
        R: Debug;

    fn poll_io<R, F>(&self, token: Token, interests: Interest, mut f: F) -> Poll<io::Result<R>>
    where
        F: FnMut() -> io::Result<R>,
        R: Debug,
    {
        log::trace!("io token={:?} {:?} poll", token, interests);

        loop {
            match f() {
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    return Poll::Pending;
                }
                Err(err) if err.kind() == io::ErrorKind::Interrupted => {
                    log::trace!("io token={:?} {:?} Interrupted", token, interests);
                    continue;
                }
                output => {
                    log::trace!("io token={:?} {:?} Success", token, interests);

                    return Poll::Ready(output);
                }
            }
        }
    }

    /// Run io device event loop forever
    fn event_loop(&self, poll_timeout: Option<Duration>) -> io::Result<()> {
        loop {
            self.event_loop_once(poll_timeout)?;
        }
    }

    fn event_loop_once(&self, poll_timeout: Option<Duration>) -> io::Result<()>;
}

pub trait SelectableIoDevice: IoDevice {
    // Select one ready io to invoke and register `waker` if all IOs returns `WOULD_BLOCK`
    fn poll_select<R, F>(
        &self,
        cx: &mut Context<'_>,
        tokens: &[Token],
        interests: Interest,
        f: F,
    ) -> Poll<io::Result<R>>
    where
        F: FnMut(Token) -> io::Result<R>,
        R: Debug;
}

/// The trait to get context bound [`IoDevice`] object
pub trait ContextIoDevice: IoDevice {
    fn get() -> Self
    where
        Self: Sized;
}

pub trait STRunner: IoDevice + ContextIoDevice {
    fn run_loop<'a, Spawner>(spawner: Spawner, poll_timeout: Option<Duration>) -> io::Result<()>
    where
        Spawner: Fn(BoxFuture<'static, ()>) + Send + Clone + 'static,
        Self: Sized + Send + 'static,
    {
        let local_spawner = spawner.clone();

        let inner = async move {
            let io: Self = Self::get();
            io.event_loop_once(poll_timeout).unwrap();
            Self::run_loop::<Spawner>(local_spawner, poll_timeout).unwrap();
        };

        spawner(Box::pin(inner));

        Ok(())
    }
}

pub trait MTRunner: IoDevice {
    /// Start MultiThread IoDevice service.
    fn run_loop(&self, poll_timeout: Option<Duration>)
    where
        Self: ContextIoDevice + Sized,
    {
        std::thread::spawn(move || {
            _ = Self::get().event_loop(poll_timeout);
        });
    }
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

    /// Create a new asynchronous io calling with nonblock sync `F`
    fn async_select<'a, F>(
        &'a self,
        tokens: &'a [Token],
        interests: Interest,
        f: F,
    ) -> SelectIo<'_, Self, F>
    where
        Self: SelectableIoDevice + Sized,
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

pub struct SelectIo<'a, IO, F> {
    io: &'a IO,
    tokens: &'a [Token],
    interests: Interest,
    f: F,
}

impl<'a, IO, F, R> Future for SelectIo<'a, IO, F>
where
    IO: IoDevice + SelectableIoDevice,
    F: FnMut(Token) -> io::Result<R> + Unpin,
    R: Debug,
{
    type Output = io::Result<R>;

    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.io
            .poll_select(cx, self.tokens, self.interests, &mut self.f)
    }
}

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
    R: Debug,
{
    type Output = io::Result<R>;

    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.io
            .poll_io_with_context(cx, self.token, self.interests, &mut self.f)
    }
}

struct MioDeviceInner {
    /// readable waker collection
    read_wakers: HashMap<Token, WakerOrToken>,
    /// writable waker collection
    write_wakers: HashMap<Token, WakerOrToken>,

    read_group: HashMap<Token, (Waker, Vec<Token>)>,

    write_group: HashMap<Token, (Waker, Vec<Token>)>,
}

impl MioDeviceInner {
    fn new() -> io::Result<Self> {
        Ok(Self {
            read_wakers: Default::default(),
            write_wakers: Default::default(),
            read_group: Default::default(),
            write_group: Default::default(),
        })
    }

    fn register_waker(
        &mut self,
        cx: &mut Context<'_>,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        log::trace!("io_device register io={:?} {:?}", token, interests,);

        if interests.is_readable() {
            self.read_wakers.insert(token, cx.waker().clone().into());
        }

        if interests.is_writable() {
            self.write_wakers.insert(token, cx.waker().clone().into());
        }

        Ok(())
    }

    fn deregister_waker(&mut self, token: Token, interests: Interest) -> io::Result<()> {
        log::trace!("io_device deregister io={:?} {:?}", token, interests,);

        if interests.is_readable() {
            self.read_wakers.remove(&token);
        }

        if interests.is_writable() {
            self.write_wakers.remove(&token);
        }

        Ok(())
    }

    fn register_wakers(&mut self, cx: &mut Context<'_>, tokens: &[Token], interests: Interest) {
        log::trace!(
            "io_device register group={:?} {:?} {:?}",
            tokens[0],
            interests,
            tokens
        );

        for token in tokens {
            if interests.is_readable() {
                self.read_wakers.insert(*token, tokens[0].into());
            }

            if interests.is_writable() {
                self.write_wakers.insert(*token, tokens[0].into());
            }
        }

        if interests.is_readable() {
            self.read_group
                .insert(tokens[0], (cx.waker().clone(), tokens.to_owned()));
        }

        if interests.is_writable() {
            self.write_group
                .insert(tokens[0], (cx.waker().clone(), tokens.to_owned()));
        }
    }

    fn deregister_wakers(&mut self, tokens: &[Token], interests: Interest) {
        log::trace!(
            "io_device deregister group={:?} {:?} {:?}",
            tokens[0],
            interests,
            tokens
        );

        for token in tokens {
            if interests.is_readable() {
                self.read_wakers.remove(token);
            }

            if interests.is_writable() {
                self.write_wakers.remove(token);
            }
        }

        if interests.is_readable() {
            self.read_group.remove(&tokens[0]);
        }

        if interests.is_writable() {
            self.write_group.remove(&tokens[0]);
        }
    }

    fn handle_events(&mut self, events: &Events) {
        for event in events {
            self.handle_event(event);
        }
    }

    fn handle_event(&mut self, event: &Event) -> bool {
        log::trace!("io_device handle {:?}", event);

        if event.is_readable() {
            if let Some(waker_or_token) = self.read_wakers.remove(&event.token()) {
                log::trace!("io_device found read_waker {:?}", waker_or_token);

                match waker_or_token {
                    WakerOrToken::Token(token) => {
                        if let Some((waker, tokens)) = self.read_group.remove(&token) {
                            for token in tokens {
                                self.read_wakers.remove(&token);
                            }

                            waker.wake_by_ref();

                            log::trace!(
                                "io_device handle group={:?} Readable event",
                                waker_or_token
                            );

                            return true;
                        }
                    }
                    WakerOrToken::Waker(waker) => {
                        waker.wake_by_ref();

                        log::trace!("io_device handle token={:?} Readable event", event.token());
                    }
                }
            }
        }

        if event.is_writable() {
            if let Some(waker_or_token) = self.write_wakers.remove(&event.token()) {
                match waker_or_token {
                    WakerOrToken::Token(token) => {
                        if let Some((waker, tokens)) = self.write_group.remove(&token) {
                            for token in tokens {
                                self.read_wakers.remove(&token);
                            }

                            waker.wake_by_ref();

                            log::trace!(
                                "io_device handle group={:?} Writable event",
                                waker_or_token
                            );

                            return true;
                        }
                    }
                    WakerOrToken::Waker(waker) => {
                        waker.wake_by_ref();

                        log::trace!("io_device handle token={:?} Writable event", event.token());
                    }
                }
            }
        }

        return false;
    }
}

#[derive(Debug)]
enum WakerOrToken {
    Waker(Waker),
    Token(Token),
}

impl From<Waker> for WakerOrToken {
    fn from(value: Waker) -> Self {
        Self::Waker(value)
    }
}

impl From<Token> for WakerOrToken {
    fn from(value: Token) -> Self {
        Self::Token(value)
    }
}

/// IoDevice using mio implementation
pub struct BasicMioDevice<TM: ThreadModel = MTModel> {
    /// handle poll_select wakers
    inner: TM::Guard<MioDeviceInner>,
    poll: TM::Guard<mio::Poll>,
    next_token: Arc<AtomicUsize>,
}

impl<TM: ThreadModel> Clone for BasicMioDevice<TM> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            poll: self.poll.clone(),
            next_token: self.next_token.clone(),
        }
    }
}

impl<TM: ThreadModel> BasicMioDevice<TM> {
    /// Create new io device object.
    #[track_caller]
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            inner: MioDeviceInner::new()?.into(),
            poll: mio::Poll::new()?.into(),
            next_token: Default::default(),
        })
    }

    fn new_token(&self) -> Token {
        use std::sync::atomic::Ordering;

        let next_token = self.next_token.fetch_add(1, Ordering::SeqCst);

        let token = Token(next_token);

        token
    }
}

impl<TM: ThreadModel> IoDevice for BasicMioDevice<TM> {
    type Guard<T> = TM::Guard<T>;

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

        self.inner
            .get_mut()
            .deregister_waker(old_token, interests)?;

        Ok(token)
    }

    fn deregister<S>(&self, source: &mut S, token: Token) -> io::Result<()>
    where
        S: Source,
    {
        self.poll.get_mut().registry().deregister(source)?;

        self.inner
            .get_mut()
            .deregister_waker(token, Interest::READABLE.add(Interest::WRITABLE))
    }

    /// Poll io events once.
    fn event_loop_once(&self, poll_timeout: Option<Duration>) -> io::Result<()> {
        let mut events = Events::with_capacity(1024);

        self.poll.get_mut().poll(
            &mut events,
            Some(poll_timeout.unwrap_or(Duration::from_micros(10))),
        )?;

        self.inner.get_mut().handle_events(&events);

        Ok(())
    }

    fn register_token_waker(
        &self,
        cx: &mut Context<'_>,
        token: &Token,
        interests: Interest,
    ) -> io::Result<()> {
        self.inner.get_mut().register_waker(cx, *token, interests)
    }

    fn deregister_token_waker(&self, token: Token, interests: Interest) -> io::Result<()> {
        self.inner.get_mut().deregister_waker(token, interests)
    }

    fn poll_io_with_context<R, F>(
        &self,
        cx: &mut Context<'_>,
        token: Token,
        interests: Interest,
        f: F,
    ) -> Poll<io::Result<R>>
    where
        F: FnMut() -> io::Result<R>,
        R: Debug,
    {
        log::trace!("io token={:?} {:?} poll", token, interests);

        self.inner.get_mut().register_waker(cx, token, interests)?;

        match self.poll_io(token, interests, f) {
            Poll::Pending => {
                log::trace!("io token={:?} {:?} WouldBlock", token, interests);

                return Poll::Pending;
            }
            polling => {
                self.inner.get_mut().deregister_waker(token, interests)?;

                polling
            }
        }
    }
}

impl<TM: ThreadModel> SelectableIoDevice for BasicMioDevice<TM> {
    fn poll_select<R, F>(
        &self,
        cx: &mut Context<'_>,
        tokens: &[Token],
        interests: Interest,
        mut f: F,
    ) -> Poll<io::Result<R>>
    where
        F: FnMut(Token) -> io::Result<R>,
        R: Debug,
    {
        let mut inner = self.inner.get_mut();

        for token in tokens {
            match self.poll_io(*token, interests, || f(*token)) {
                Poll::Pending => continue,
                polling => {
                    inner.deregister_wakers(tokens, interests);
                    return polling;
                }
            }
        }

        inner.register_wakers(cx, tokens, interests);

        Poll::Pending
    }
}
pub mod mt {
    use super::*;

    pub type MioDevice = BasicMioDevice<MTModel>;

    impl ContextIoDevice for MioDevice {
        fn get() -> Self {
            static MIODEVICEMT_INSTANCE: OnceLock<MioDevice> = OnceLock::new();

            MIODEVICEMT_INSTANCE
                .get_or_init(|| MioDevice::new().unwrap())
                .clone()
        }
    }

    impl MTRunner for MioDevice {}
}

pub mod st {

    use super::*;

    pub type MioDevice = BasicMioDevice<STModel>;

    impl ContextIoDevice for MioDevice {
        fn get() -> Self {
            thread_local! {
                static MIO_DEVICE_ST_INSTANCE: MioDevice = MioDevice::new().unwrap();
            }

            MIO_DEVICE_ST_INSTANCE.with(|io| io.clone())
        }
    }

    impl STRunner for MioDevice {}
}

#[cfg(feature = "mt")]
pub type MioDevice = mt::MioDevice;

#[cfg(all(not(feature = "mt"), feature = "st"))]
pub type MioDevice = st::MioDevice;
