use std::{io, ops::DerefMut, sync::Arc, task::Waker, time::Duration};

use dashmap::DashMap;
use hala_lockfree::timewheel::HashedTimeWheel;
use hala_sync::{Lockable, LockableNew, SpinMutex};
use mio::Poll;

use crate::{Handle, Interest, Token, TypedHandle};

use super::{timer::MioTimer, with_poller::MioWithPoller};

struct RawMioPoller {
    mio_poller: SpinMutex<mio::Poll>,
    read_wakers: DashMap<Token, Waker>,
    write_wakers: DashMap<Token, Waker>,
    registry: mio::Registry,
    hashed_timewheel: HashedTimeWheel<Token>,
    tick_duration: Duration,
}

/// [`MioPoller`] io multiplexer poller
#[derive(Clone)]
pub struct MioPoller(Arc<RawMioPoller>);

impl Drop for MioPoller {
    fn drop(&mut self) {
        if Arc::strong_count(&self.0) == 1 {
            log::trace!("Close mio poller");
        }
    }
}

impl MioPoller {
    /// Create new [`MioPoller`] with the `tick_duration` of timewheel
    pub fn new(tick_duration: Duration) -> io::Result<Self> {
        let mio_poller = Poll::new()?;

        Ok(Self(Arc::new(RawMioPoller {
            registry: mio_poller.registry().try_clone()?,
            read_wakers: Default::default(),
            write_wakers: Default::default(),
            mio_poller: SpinMutex::new(mio_poller),
            hashed_timewheel: HashedTimeWheel::new(tick_duration),
            tick_duration,
        })))
    }

    /// Poll io event and notify events waiters once, returns [`io::Error``] if any error happen.
    pub fn poll_once(&self, timeout: Option<Duration>) -> io::Result<()> {
        let timeout = timeout.unwrap_or(self.0.tick_duration);

        let mut events = mio::event::Events::with_capacity(1024);

        // first of all, poll io event.
        self.0.mio_poller.lock().poll(&mut events, Some(timeout))?;

        let mut hala_events = vec![];

        for event in events.iter() {
            let mut interests = Interest::Readable | Interest::Writable;

            if !event.is_readable() {
                interests = interests ^ Interest::Readable;
            } else if !event.is_writable() {
                interests = interests ^ Interest::Writable;
            }

            hala_events.push((Token(event.token().0), interests));
        }

        // handle timeout timers
        let timeout_timers = self.0.hashed_timewheel.next_tick();

        if let Some(timeout_timers) = timeout_timers {
            for token in timeout_timers {
                hala_events.push((token, Interest::Readable));
            }
        }

        for (token, interests) in hala_events {
            if interests.contains(Interest::Readable) {
                if let Some((_, waker)) = self.0.read_wakers.remove(&token) {
                    log::trace!("{:?}, wakeup Readable", token);
                    waker.wake();
                }
            }

            if interests.contains(Interest::Writable) {
                if let Some((_, waker)) = self.0.write_wakers.remove(&token) {
                    log::trace!("{:?}, wakeup Writable", token);
                    waker.wake();
                }
            }
        }

        Ok(())
    }

    pub fn register(&self, handle: Handle, interests: Interest) -> io::Result<()> {
        let mut mio_interests = mio::Interest::READABLE.add(mio::Interest::WRITABLE);

        if !interests.contains(Interest::Writable) {
            mio_interests = mio_interests.remove(mio::Interest::WRITABLE).unwrap();
        }

        if !interests.contains(Interest::Readable) {
            mio_interests = mio_interests.remove(mio::Interest::READABLE).unwrap();
        }

        match handle.desc {
            crate::Description::File => todo!(),
            crate::Description::TcpListener => {
                let typed_handle = TypedHandle::<MioWithPoller<mio::net::TcpListener>>::new(handle);

                typed_handle.with_mut(|obj| {
                    obj.register_poller(self.clone());

                    self.0.registry.register(
                        obj.deref_mut(),
                        mio::Token(handle.token.0),
                        mio_interests,
                    )
                })?;
            }
            crate::Description::TcpStream => {
                let typed_handle = TypedHandle::<MioWithPoller<mio::net::TcpStream>>::new(handle);

                typed_handle.with_mut(|obj| {
                    obj.register_poller(self.clone());

                    self.0.registry.register(
                        obj.deref_mut(),
                        mio::Token(handle.token.0),
                        mio_interests,
                    )
                })?;
            }
            crate::Description::UdpSocket => {
                let typed_handle = TypedHandle::<MioWithPoller<mio::net::UdpSocket>>::new(handle);

                typed_handle.with_mut(|obj| {
                    obj.register_poller(self.clone());

                    self.0.registry.register(
                        obj.deref_mut(),
                        mio::Token(handle.token.0),
                        mio_interests,
                    )
                })?;
            }
            crate::Description::Timeout => {
                let typed_handle = TypedHandle::<MioWithPoller<MioTimer>>::new(handle);

                typed_handle.with_mut(|obj| {
                    assert!(!obj.is_started(), "Call poller register twice");

                    obj.register_poller(self.clone());

                    if !obj.start(handle.token, self.0.tick_duration, &self.0.hashed_timewheel) {
                        log::trace!(
                            "timer, token={:?}, timeout={:?}, already timeout.",
                            handle.token,
                            obj.duration
                        );
                    } else {
                        log::trace!(
                            "timer, token={:?}, timeout={:?}, register successful.",
                            handle.token,
                            obj.duration
                        );
                    }
                });
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("Unsupport file handle type, {:?}", handle),
                ))
            }
        }

        Ok(())
    }

    pub(super) fn deregister(&self, handle: Handle) -> io::Result<()> {
        match handle.desc {
            crate::Description::File => todo!(),
            crate::Description::TcpListener => {
                TypedHandle::<MioWithPoller<mio::net::TcpListener>>::new(handle)
                    .with_mut(|source| self.0.registry.deregister(source.deref_mut()))?;
            }
            crate::Description::TcpStream => {
                TypedHandle::<MioWithPoller<mio::net::TcpStream>>::new(handle)
                    .with_mut(|source| self.0.registry.deregister(source.deref_mut()))?;
            }
            crate::Description::UdpSocket => {
                TypedHandle::<MioWithPoller<mio::net::UdpSocket>>::new(handle)
                    .with_mut(|source| self.0.registry.deregister(source.deref_mut()))?;
            }
            crate::Description::Timeout => TypedHandle::<MioWithPoller<MioTimer>>::new(handle)
                .with_mut(|_timer| {
                    log::trace!("timer, token={:?} deregister.", handle.token);
                }),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("[MioDriver] invalid register source: {:?}", handle),
                ))
            }
        }

        self.remove_waker(handle.token, Interest::all()).map(|_| ())
    }

    pub(super) fn add_waker(&self, token: Token, interests: Interest, waker: Waker) {
        if interests.contains(Interest::Readable) {
            self.0.read_wakers.insert(token, waker.clone());
        } else if interests.contains(Interest::Writable) {
            self.0.write_wakers.insert(token, waker.clone());
        }
    }

    pub(super) fn remove_waker(
        &self,
        token: Token,
        interests: Interest,
    ) -> io::Result<Option<Waker>> {
        if interests.contains(Interest::Readable) {
            Ok(self.0.read_wakers.remove(&token).map(|(_, waker)| waker))
        } else if interests.contains(Interest::Writable) {
            Ok(self.0.write_wakers.remove(&token).map(|(_, waker)| waker))
        } else {
            Ok(None)
        }
    }
}
