use std::{
    collections::HashMap,
    fmt::Debug,
    io,
    sync::{atomic::AtomicUsize, Arc, OnceLock},
    task::{Context, Poll, Waker},
    time::Duration,
};

use futures::{future::BoxFuture, Future};
use mio::{event::Source, Events, Interest, Token};

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

    /// invoke one poll io
    fn poll_io<R, F>(
        &self,
        cx: &mut Context<'_>,
        token: Token,
        interests: Interest,
        f: F,
    ) -> Poll<io::Result<R>>
    where
        F: FnMut() -> io::Result<R>,
        R: Debug;

    fn event_loop(&self, poll_timeout: Option<Duration>) -> io::Result<()>;

    fn event_loop_once(&self, poll_timeout: Option<Duration>) -> io::Result<()>;
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
    R: Debug,
{
    type Output = io::Result<R>;

    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.io.poll_io(cx, self.token, self.interests, &mut self.f)
    }
}

/// IoDevice using mio implementation
pub struct BasicMioDevice<TM: ThreadModel = MTModel> {
    poll: TM::Guard<mio::Poll>,
    /// readable waker collection
    read_wakers: TM::Guard<HashMap<Token, Waker>>,
    /// writable waker collection
    write_wakers: TM::Guard<HashMap<Token, Waker>>,
    /// token generating seed
    next_token: Arc<AtomicUsize>,
}

impl<TM: ThreadModel> Clone for BasicMioDevice<TM> {
    fn clone(&self) -> Self {
        Self {
            poll: self.poll.clone(),
            read_wakers: self.read_wakers.clone(),
            write_wakers: self.write_wakers.clone(),
            next_token: self.next_token.clone(),
        }
    }
}

impl<TM: ThreadModel> BasicMioDevice<TM> {
    /// Create new io device object.
    #[track_caller]
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
        log::trace!("io token={:?} {:?} poll", token, interests);

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

                    log::trace!("io token={:?} {:?} WouldBlock", token, interests);
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

    /// Poll io events once.
    fn event_loop_once(&self, poll_timeout: Option<Duration>) -> io::Result<()> {
        let mut events = Events::with_capacity(1024);

        self.poll.get_mut().poll(
            &mut events,
            Some(poll_timeout.unwrap_or(Duration::from_micros(1))),
        )?;

        for event in events.iter() {
            log::trace!("io_device raised event {:?}", event);

            if event.is_readable() {
                if let Some(waker) = self.read_wakers.get_mut().remove(&event.token()) {
                    log::trace!("io {:?} readable wake", event.token());
                    waker.wake_by_ref();
                }
            }

            if event.is_writable() {
                if let Some(waker) = self.write_wakers.get_mut().remove(&event.token()) {
                    log::trace!("io {:?} writable wake", event.token());
                    waker.wake_by_ref();
                }
            }
        }

        Ok(())
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
