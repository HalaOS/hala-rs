use std::{
    collections::VecDeque,
    fmt::Debug,
    io,
    sync::Arc,
    task::Poll,
    time::{Duration, Instant},
};

use sync_traits::{Shared, LocalShared, MutexShared};
use timewheel::TimeWheel;

use crate::{Handle, Interest, Token, TypedHandle};

use super::WithPoller;

fn duration_to_ticks(duration: Duration, tick_duration: Duration, round_up: bool) -> u128 {
    let duration_m = duration.as_nanos();
    let tick_duration_m = tick_duration.as_nanos();

    let mut ticks = duration_m / tick_duration_m;

    if round_up && tick_duration * (ticks as u32) < duration {
        ticks += 1;
    }

    ticks
}

fn adjust_timeout(
    time_wheel_start_time: Instant,
    time_wheel_ticks: u128,
    tick_duration: Duration,
    timeout: Duration,
) -> Duration {

    let ticks = time_wheel_start_time.elapsed().as_nanos() / tick_duration.as_nanos();

    if ticks > time_wheel_ticks {
        return timeout + tick_duration * (ticks - time_wheel_ticks) as u32;
    }

    return timeout;
}

pub(crate) struct MioTimeout {
    pub(crate) duration: Duration,
    pub(crate) start_time: Option<Instant>,
    pub(crate) slot: Option<u128>,
    pub(crate) tick_duration: Option<Duration>,
}

impl Debug for MioTimeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(start_time) = self.start_time.as_ref() {
            write!(
                f,
                "duration={:?}, slot={}, elapsed={:?}",
                self.duration,
                self.slot.unwrap(),
                start_time.elapsed()
            )
        } else {
            write!(f, "duration={:?}", self.duration)
        }
    }
}

impl MioTimeout {
    pub(crate) fn new(duration: Duration) -> Self {
        Self {
            duration,
            start_time: None,
            slot: None,
            tick_duration: None,
        }
    }

    pub(crate) fn is_register(&self) -> bool {
        self.start_time.is_some() && self.slot.is_some()
    }

    pub(crate) fn is_expired(&self) -> bool {
        if let Some(start_time) = self.start_time {
            let elapsed = start_time.elapsed();

            if elapsed >= self.duration {
                return true;
            }

            if self.duration - elapsed < self.tick_duration.unwrap() {
                return true;
            }

            return false;
        } else {
            false
        }
    }
}

pub(crate) struct MioTimeWheel {
    time_wheel: TimeWheel<Token>,
    last_tick: Instant,
}

impl MioTimeWheel {
    fn new() -> Self {
        Self {
            time_wheel: TimeWheel::new(2048),
            last_tick: Instant::now(),
        }
    }

    fn tick(&mut self, tick_duration: Duration) -> VecDeque<Token> {
        let elapsed = self.last_tick.elapsed();

        let elapsed_ticks = duration_to_ticks(elapsed, tick_duration, false);

        if elapsed_ticks <= self.time_wheel.tick {
            return VecDeque::new();
        }

        let ticks = elapsed_ticks - self.time_wheel.tick;

        let mut timeout_vec = VecDeque::new();

        for _ in 0..ticks {
            if let Poll::Ready(mut v) = self.time_wheel.tick() {
                for token in &v {
                    log::trace!(
                        "{:?} expired, time_wheel_elapsed_ticks={},time_wheel_ticks={}, time_wheel_elapsed={:?}",
                        token,
                        elapsed_ticks,
                        self.time_wheel.tick,
                        elapsed
                    )
                }

                timeout_vec.append(&mut v);
            }
        }

        timeout_vec
    }
}

pub struct MioPoller<Poll,TimeWheel> {
    poll: Poll,
    registry: Arc<mio::Registry>,
    time_wheel: TimeWheel,
    tick_duration: Duration,
}

impl<Poll,TimeWheel> Clone for MioPoller<Poll,TimeWheel>
where
    Poll: Clone,
    TimeWheel: Clone
{
    fn clone(&self) -> Self {
        Self {
            poll: self.poll.clone(),
            registry: self.registry.clone(),
            time_wheel: self.time_wheel.clone(),
            tick_duration: self.tick_duration.clone(),
        }
    }
}

impl<Poll,TimeWheel> Default for MioPoller<Poll,TimeWheel>
where
    Poll: Shared<Value = mio::Poll> + From<mio::Poll>,
    TimeWheel: Shared<Value = MioTimeWheel> + From<MioTimeWheel>,
{
    fn default() -> Self {
        let poll = mio::Poll::new().unwrap();

        let registry = poll.registry().try_clone().unwrap();

        Self {
            poll: poll.into(),
            registry: registry.into(),
            time_wheel: MioTimeWheel::new().into(),
            tick_duration: Duration::from_millis(1),
        }
    }
}

impl<Poll,TimeWheel> MioPoller<Poll,TimeWheel>
where
    Poll: Shared<Value = mio::Poll>,
    TimeWheel: Shared<Value = MioTimeWheel>
{
   pub fn poll_once(&self, timeout: Option<Duration>) -> io::Result<Vec<(Token, Interest)>> {
        let poll_timeout = timeout.unwrap_or(self.tick_duration);

        let mut events = mio::event::Events::with_capacity(1024);

        self.poll.lock_mut().poll(&mut events, Some(poll_timeout))?;

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

        let timeout = self.time_wheel.lock_mut().tick(self.tick_duration);

        for token in timeout {
            hala_events.push((token, Interest::Readable));
        }

        Ok(hala_events)
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
            crate::Description::TcpListener => TypedHandle::<WithPoller<mio::net::TcpListener>>::new(handle)
                .with_mut(|source| {
                    self.registry
                        .register(&mut source.value, mio::Token(handle.token.0), mio_interests)
                }),
            crate::Description::TcpStream => TypedHandle::<WithPoller<mio::net::TcpStream>>::new(handle)
                .with_mut(|source| {
                    self.registry
                        .register(&mut source.value, mio::Token(handle.token.0), mio_interests)
                }),
            crate::Description::UdpSocket => TypedHandle::<WithPoller<mio::net::UdpSocket>>::new(handle)
                .with_mut(|source| {
                    self.registry
                        .register(&mut source.value, mio::Token(handle.token.0), mio_interests)
                }),
            crate::Description::Timeout => {
                TypedHandle::<WithPoller<MioTimeout>>::new(handle).with_mut(|timeout| {
                    if timeout.duration.is_zero() {
                        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Timeout is zero"));
                    }
                    
                    timeout.start_time = Some(Instant::now());
                    timeout.tick_duration = Some(self.tick_duration);

                    let mut time_wheel = self.time_wheel.lock_mut();

                    let timeout_duration = adjust_timeout(time_wheel.last_tick,time_wheel.time_wheel.tick,self.tick_duration,timeout.duration);

                    let ticks = duration_to_ticks(timeout_duration, self.tick_duration, true);

                    let slot = time_wheel.time_wheel.add(ticks, handle.token);

                    timeout.slot = Some(slot);

                    log::trace!(
                        "{:?} register timeout {:?}, slot={}, tick_duration={:?}, ticks={},time_wheel_elapsed={:?}, time_wheel_ticks={}, time_wheel_steps={}",
                        handle.token,
                        timeout.value,
                        timeout.slot.unwrap(),
                        self.tick_duration,
                        ticks,
                        time_wheel.last_tick.elapsed(),
                        time_wheel.time_wheel.tick,
                        time_wheel.time_wheel.steps
                    );

                    Ok(())
                })
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("[MioDriver] invalid register source: {:?}", handle),
                ))
            }
        }
    }

   pub fn reregister(&self, handle: Handle, interests: Interest) -> io::Result<()> {
        let mut mio_interests = mio::Interest::READABLE.add(mio::Interest::WRITABLE);

        if !interests.contains(Interest::Writable) {
            mio_interests = mio_interests.remove(mio::Interest::WRITABLE).unwrap();
        }

        if !interests.contains(Interest::Readable) {
            mio_interests = mio_interests.remove(mio::Interest::READABLE).unwrap();
        }

        match handle.desc {
            crate::Description::File => todo!(),
            crate::Description::TcpListener => TypedHandle::<WithPoller<mio::net::TcpListener>>::new(handle)
                .with_mut(|source| {
                    self.registry
                        .reregister(&mut source.value, mio::Token(handle.token.0), mio_interests)
                }),
            crate::Description::TcpStream => TypedHandle::<WithPoller<mio::net::TcpStream>>::new(handle)
                .with_mut(|source| {
                    self.registry
                        .reregister(&mut source.value, mio::Token(handle.token.0), mio_interests)
                }),
            crate::Description::UdpSocket => TypedHandle::<WithPoller<mio::net::UdpSocket>>::new(handle)
                .with_mut(|source| {
                    self.registry
                        .reregister(&mut source.value, mio::Token(handle.token.0), mio_interests)
                }),
            crate::Description::Timeout => {
                TypedHandle::<WithPoller<MioTimeout>>::new(handle).with_mut(|timeout| {
                    if timeout.duration.is_zero() {
                        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Timeout is zero"));
                    }

                    let mut time_wheel = self.time_wheel.lock_mut();

                    if let Some(slot) = timeout.slot {
                        time_wheel.time_wheel.remove(slot, handle.token);
                    }

                    let timeout_duration = adjust_timeout(
                        time_wheel.last_tick,
                        time_wheel.time_wheel.tick,
                        self.tick_duration,
                        timeout.duration,
                    );

                    let ticks = duration_to_ticks(timeout_duration, self.tick_duration, true);

                    let slot = time_wheel.time_wheel.add(ticks, handle.token);

                    timeout.start_time = Some(Instant::now());

                    timeout.slot = Some(slot);
                    timeout.tick_duration = Some(self.tick_duration);

                    Ok(())
                })
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("[MioDriver] invalid register source: {:?}", handle),
                ));
            }
        }
    }
   pub fn deregister(&self, handle: Handle) -> io::Result<()> {
        match handle.desc {
            crate::Description::File => todo!(),
            crate::Description::TcpListener => TypedHandle::<WithPoller<mio::net::TcpListener>>::new(handle)
                .with_mut(|source| self.registry.deregister(&mut source.value)),
            crate::Description::TcpStream => TypedHandle::<WithPoller<mio::net::TcpStream>>::new(handle)
                .with_mut(|source| self.registry.deregister(&mut source.value)),
            crate::Description::UdpSocket => TypedHandle::<WithPoller<mio::net::UdpSocket>>::new(handle)
                .with_mut(|source| self.registry.deregister(&mut source.value)),
            crate::Description::Timeout => {
                TypedHandle::<WithPoller<MioTimeout>>::new(handle).with_mut(|timeout| {
                    assert!(!timeout.duration.is_zero());

                    let mut time_wheel = self.time_wheel.lock_mut();

                    if let Some(slot) = timeout.slot {
                        time_wheel.time_wheel.remove(slot, handle.token);
                    }

                    Ok(())
                })
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("[MioDriver] invalid register source: {:?}", handle),
                ))
            }
        }
    }
}


pub type LocalMioPoller = MioPoller<LocalShared<mio::Poll>,LocalShared<MioTimeWheel>>;

pub type MutexMioPoller = MioPoller<MutexShared<mio::Poll>,MutexShared<MioTimeWheel>>;