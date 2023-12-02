use std::{io, sync::Arc, time::Duration};

use crate::{Handle, Interest, TypedHandle};

use super::*;

pub trait MioPoller {
    fn poll_once(&self, timeout: Option<Duration>) -> io::Result<()>;
    fn register(&self, handle: Handle, interests: Interest) -> io::Result<()>;

    fn reregister(&self, handle: Handle, interests: Interest) -> io::Result<()>;

    fn deregister(&self, handle: Handle) -> io::Result<()>;
}

pub struct BasicMioPoller<TM: ThreadModel> {
    poll: TM::Guard<mio::Poll>,
    registry: Arc<mio::Registry>,
}

impl<TM> Clone for BasicMioPoller<TM>
where
    TM: ThreadModel,
{
    fn clone(&self) -> Self {
        Self {
            poll: self.poll.clone(),
            registry: self.registry.clone(),
        }
    }
}

impl<TM> Default for BasicMioPoller<TM>
where
    TM: ThreadModel,
{
    fn default() -> Self {
        let poll = mio::Poll::new().unwrap();

        let registry = poll.registry().try_clone().unwrap();

        Self {
            poll: poll.into(),
            registry: registry.into(),
        }
    }
}

impl<TM> MioPoller for BasicMioPoller<TM>
where
    TM: ThreadModel,
{
    fn poll_once(&self, timeout: Option<Duration>) -> io::Result<()> {
        let mut events = mio::event::Events::with_capacity(1024);

        self.poll.get_mut().poll(&mut events, timeout)
    }

    fn register(&self, handle: Handle, interests: Interest) -> io::Result<()> {
        let mut mio_interests = mio::Interest::READABLE.add(mio::Interest::WRITABLE);

        if !interests.contains(Interest::Writable) {
            mio_interests = mio_interests.remove(mio::Interest::WRITABLE).unwrap();
        }

        if !interests.contains(Interest::Readable) {
            mio_interests = mio_interests.remove(mio::Interest::READABLE).unwrap();
        }

        match handle.desc {
            crate::Description::File => todo!(),
            crate::Description::TcpListener => TypedHandle::<mio::net::TcpListener>::new(handle)
                .with_mut(|source| {
                    self.registry
                        .register(source, mio::Token(handle.token.0), mio_interests)
                }),
            crate::Description::TcpStream => TypedHandle::<mio::net::TcpStream>::new(handle)
                .with_mut(|source| {
                    self.registry
                        .register(source, mio::Token(handle.token.0), mio_interests)
                }),
            crate::Description::UdpSocket => TypedHandle::<mio::net::UdpSocket>::new(handle)
                .with_mut(|source| {
                    self.registry
                        .register(source, mio::Token(handle.token.0), mio_interests)
                }),
            crate::Description::Tick => todo!(),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("[MioDriver] invalid register source: {:?}", handle),
                ))
            }
        }
    }

    fn reregister(&self, handle: Handle, interests: Interest) -> io::Result<()> {
        let mut mio_interests = mio::Interest::READABLE.add(mio::Interest::WRITABLE);

        if !interests.contains(Interest::Writable) {
            mio_interests = mio_interests.remove(mio::Interest::WRITABLE).unwrap();
        }

        if !interests.contains(Interest::Readable) {
            mio_interests = mio_interests.remove(mio::Interest::READABLE).unwrap();
        }

        match handle.desc {
            crate::Description::File => todo!(),
            crate::Description::TcpListener => TypedHandle::<mio::net::TcpListener>::new(handle)
                .with_mut(|source| {
                    self.registry
                        .reregister(source, mio::Token(handle.token.0), mio_interests)
                }),
            crate::Description::TcpStream => TypedHandle::<mio::net::TcpStream>::new(handle)
                .with_mut(|source| {
                    self.registry
                        .reregister(source, mio::Token(handle.token.0), mio_interests)
                }),
            crate::Description::UdpSocket => TypedHandle::<mio::net::UdpSocket>::new(handle)
                .with_mut(|source| {
                    self.registry
                        .reregister(source, mio::Token(handle.token.0), mio_interests)
                }),
            crate::Description::Tick => todo!(),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("[MioDriver] invalid register source: {:?}", handle),
                ));
            }
        }
    }
    fn deregister(&self, handle: Handle) -> io::Result<()> {
        match handle.desc {
            crate::Description::File => todo!(),
            crate::Description::TcpListener => TypedHandle::<mio::net::TcpListener>::new(handle)
                .with_mut(|source| self.registry.deregister(source)),
            crate::Description::TcpStream => TypedHandle::<mio::net::TcpStream>::new(handle)
                .with_mut(|source| self.registry.deregister(source)),
            crate::Description::UdpSocket => TypedHandle::<mio::net::UdpSocket>::new(handle)
                .with_mut(|source| self.registry.deregister(source)),
            crate::Description::Tick => todo!(),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("[MioDriver] invalid register source: {:?}", handle),
                ))
            }
        }
    }
}

pub type STMioPoller = BasicMioPoller<STModel>;

pub type MTMioPoller = BasicMioPoller<MTModel>;
