use std::{
    io::{self, Read, Write},
    ops,
    task::Waker,
    time::Duration,
};

use crate::{
    Description, Driver, Handle, Interest, IntoRawDriver, RawDriverExt, Token, TypedHandle,
};

use super::{
    notifier::{LocalMioNotifier, MutexMioNotifier},
    *,
};

struct MioPollerWithNotifier {
    local: bool,
    data: *const (),
}

impl MioPollerWithNotifier {
    fn new(local: bool) -> Self {
        if local {
            let data = Box::into_raw(Box::new((
                LocalMioPoller::default(),
                LocalMioNotifier::default(),
            ))) as *const ();

            return MioPollerWithNotifier { local, data };
        } else {
            let data = Box::into_raw(Box::new((
                MutexMioPoller::default(),
                MutexMioNotifier::default(),
            ))) as *const ();

            return MioPollerWithNotifier { local, data };
        }
    }

    fn with_local<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&LocalMioPoller, &LocalMioNotifier) -> R,
    {
        let boxed = unsafe { Box::from_raw(self.data as *mut (LocalMioPoller, LocalMioNotifier)) };

        let r = f(&boxed.0, &boxed.1);

        Box::into_raw(boxed);

        r
    }

    fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&MutexMioPoller, &MutexMioNotifier) -> R,
    {
        let boxed = unsafe { Box::from_raw(self.data as *mut (MutexMioPoller, MutexMioNotifier)) };

        let r = f(&boxed.0, &boxed.1);

        Box::into_raw(boxed);

        r
    }

    fn add_waker(&self, token: Token, interests: Interest, waker: Waker) {
        if self.local {
            self.with_local(|_, notifier| notifier.add_waker(token, interests, waker))
        } else {
            self.with(|_, notifier| notifier.add_waker(token, interests, waker))
        }
    }

    fn remove_waker(&self, token: Token, interests: Interest) -> io::Result<Option<Waker>> {
        if self.local {
            self.with_local(|_, notifier| notifier.remove_waker(token, interests))
        } else {
            self.with(|_, notifier| notifier.remove_waker(token, interests))
        }
    }

    fn register(&self, handle: Handle, interests: Interest) -> io::Result<()> {
        if self.local {
            self.with_local(|poller, _| poller.register(handle, interests))
        } else {
            self.with(|poller, _| poller.register(handle, interests))
        }
    }

    fn reregister(&self, handle: Handle, interests: Interest) -> io::Result<()> {
        if self.local {
            self.with_local(|poller, _| poller.reregister(handle, interests))
        } else {
            self.with(|poller, _| poller.reregister(handle, interests))
        }
    }

    fn deregister(&self, handle: Handle) -> io::Result<()> {
        if self.local {
            self.with_local(|poller, _| poller.deregister(handle))
        } else {
            self.with(|poller, _| poller.deregister(handle))
        }
    }

    fn poll_once(&self, timeout: Option<Duration>) -> io::Result<Vec<(Token, Interest)>> {
        if self.local {
            self.with_local(|poller, _| poller.poll_once(timeout))
        } else {
            self.with(|poller, _| poller.poll_once(timeout))
        }
    }

    fn on(&self, token: Token, interests: Interest) {
        if self.local {
            self.with_local(|_, notifier| notifier.on(token, interests))
        } else {
            self.with(|_, notifier| notifier.on(token, interests))
        }
    }
}

impl Clone for MioPollerWithNotifier {
    fn clone(&self) -> Self {
        if self.local {
            let data = self.with_local(|poller, notifer| {
                Box::into_raw(Box::new((poller.clone(), notifer.clone()))) as *const ()
            });

            return Self {
                local: self.local,
                data,
            };
        } else {
            let data = self.with(|poller, notifer| {
                Box::into_raw(Box::new((poller.clone(), notifer.clone()))) as *const ()
            });

            return Self {
                local: self.local,
                data,
            };
        }
    }
}

impl Drop for MioPollerWithNotifier {
    fn drop(&mut self) {
        if self.local {
            _ = unsafe { Box::from_raw(self.data as *mut (LocalMioPoller, LocalMioNotifier)) };
        } else {
            _ = unsafe { Box::from_raw(self.data as *mut (MutexMioPoller, MutexMioNotifier)) };
        }
    }
}

struct WithPoller<T> {
    value: T,
    poller: Option<MioPollerWithNotifier>,
}

impl<T> WithPoller<T> {
    fn new(value: T) -> Self {
        Self {
            value,
            poller: None,
        }
    }

    fn get_poller(&self) -> io::Result<&MioPollerWithNotifier> {
        self.poller.as_ref().ok_or(io::Error::new(
            io::ErrorKind::InvalidData,
            "Call poll register first",
        ))
    }
}

impl<T> ops::Deref for WithPoller<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T> ops::DerefMut for WithPoller<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

pub struct MioDriver {}

impl MioDriver {
    pub fn new() -> Self {
        Self {}
    }
}

impl Clone for MioDriver {
    fn clone(&self) -> Self {
        Self {}
    }
}

impl MioDriver {
    fn nonblocking_call<R, F>(
        &self,
        poller: &MioPollerWithNotifier,
        token: Token,
        interests: Interest,
        waker: Waker,
        mut f: F,
    ) -> io::Result<R>
    where
        F: FnMut() -> io::Result<R>,
    {
        poller.add_waker(token, interests, waker);

        loop {
            match f() {
                Err(err) if err.kind() == io::ErrorKind::Interrupted => {
                    continue;
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => return Err(err),

                r => {
                    _ = poller.remove_waker(token, interests);

                    return r;
                }
            }
        }
    }
}

impl RawDriverExt for MioDriver {
    fn fd_user_define_open(&self, id: usize, _buf: &[u8]) -> std::io::Result<crate::Handle> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("[MioDriver] unspport userdefined file description({})", id),
        ))
    }

    fn fd_user_define_close(&self, id: usize, _handle: crate::Handle) -> std::io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("[MioDriver] unspport userdefined file description({})", id),
        ))
    }

    fn fd_user_define_clone(&self, handle: crate::Handle) -> std::io::Result<crate::Handle> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "[MioDriver:fd_user_define_clone] unspport userdefined file description,{:?}",
                handle
            ),
        ))
    }

    fn file_open(&self, _path: &str, _mode: crate::FileMode) -> std::io::Result<crate::Handle> {
        unimplemented!("Unimplement file_open")
    }

    fn file_write(
        &self,
        _waker: Waker,
        _handle: crate::Handle,
        _buf: &[u8],
    ) -> std::io::Result<usize> {
        unimplemented!("Unimplement file_write")
    }

    fn file_read(
        &self,
        _waker: Waker,
        _handle: crate::Handle,
        _buf: &mut [u8],
    ) -> std::io::Result<usize> {
        unimplemented!("Unimplement file_write")
    }

    fn file_close(&self, _handle: crate::Handle) -> std::io::Result<()> {
        unimplemented!("Unimplement file_close")
    }

    fn timeout_open(&self, duration: std::time::Duration) -> std::io::Result<crate::Handle> {
        Ok((
            Description::Timeout,
            WithPoller::new(MioTimeout::new(duration)),
        )
            .into())
    }

    fn timeout(&self, waker: Waker, handle: crate::Handle) -> std::io::Result<bool> {
        handle.expect(Description::Timeout)?;

        TypedHandle::<WithPoller<MioTimeout>>::new(handle).with(|timeout| {
            if !timeout.is_register() {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Call fd_cntl/Register first",
                ));
            }

            log::debug!("{:?}", timeout.value);

            if timeout.is_expired() {
                return Ok(true);
            }

            timeout
                .get_poller()?
                .add_waker(handle.token, Interest::Readable, waker);

            Ok(false)
        })
    }

    fn timeout_close(&self, handle: crate::Handle) -> std::io::Result<()> {
        handle.expect(Description::Timeout)?;

        handle.drop_as::<WithPoller<MioTimeout>>();

        Ok(())
    }

    fn tcp_listener_bind(&self, laddrs: &[std::net::SocketAddr]) -> std::io::Result<crate::Handle> {
        let tcp_listener = std::net::TcpListener::bind(laddrs)?;

        tcp_listener.set_nonblocking(true)?;

        let tcp_lisener = mio::net::TcpListener::from_std(tcp_listener);

        Ok((Description::TcpListener, WithPoller::new(tcp_lisener)).into())
    }

    fn tcp_listener_accept(
        &self,
        waker: Waker,
        handle: crate::Handle,
    ) -> std::io::Result<(crate::Handle, std::net::SocketAddr)> {
        log::trace!("[MioDriver] TcpListener accept, {:?}", handle);

        handle.expect(Description::TcpListener)?;

        let typed_handle = TypedHandle::<WithPoller<mio::net::TcpListener>>::new(handle);

        typed_handle.with(|socket| {
            self.nonblocking_call(
                socket.get_poller()?,
                handle.token,
                Interest::Readable,
                waker,
                || socket.accept(),
            )
            .map(|(stream, raddr)| {
                (
                    (Description::TcpStream, WithPoller::new(stream)).into(),
                    raddr,
                )
            })
        })
    }

    fn tcp_listener_close(&self, handle: crate::Handle) -> std::io::Result<()> {
        handle.expect(Description::TcpListener)?;

        handle.drop_as::<WithPoller<mio::net::TcpListener>>();

        Ok(())
    }

    fn tcp_stream_connect(
        &self,
        raddrs: &[std::net::SocketAddr],
    ) -> std::io::Result<crate::Handle> {
        let tcp_stream = std::net::TcpStream::connect(raddrs)?;

        tcp_stream.set_nonblocking(true)?;

        let tcp_stream = mio::net::TcpStream::from_std(tcp_stream);

        Ok((Description::TcpStream, WithPoller::new(tcp_stream)).into())
    }

    fn tcp_stream_write(
        &self,
        waker: Waker,
        handle: crate::Handle,
        buf: &[u8],
    ) -> std::io::Result<usize> {
        handle.expect(Description::TcpStream)?;

        let typed_handle = TypedHandle::<WithPoller<mio::net::TcpStream>>::new(handle);

        typed_handle.with_mut(|socket| {
            let poller = socket.get_poller()?.clone();
            self.nonblocking_call(&poller, handle.token, Interest::Writable, waker, || {
                socket.write(buf)
            })
        })
    }

    fn tcp_stream_read(
        &self,
        waker: Waker,
        handle: crate::Handle,
        buf: &mut [u8],
    ) -> std::io::Result<usize> {
        handle.expect(Description::TcpStream)?;

        let typed_handle = TypedHandle::<WithPoller<mio::net::TcpStream>>::new(handle);

        typed_handle.with_mut(|socket| {
            let poller = socket.get_poller()?.clone();

            self.nonblocking_call(&poller, handle.token, Interest::Readable, waker, || {
                socket.read(buf)
            })
        })
    }

    fn tcp_stream_close(&self, handle: crate::Handle) -> std::io::Result<()> {
        handle.expect(Description::TcpStream)?;

        handle.drop_as::<WithPoller<mio::net::TcpStream>>();

        Ok(())
    }

    fn udp_socket_bind(&self, laddrs: &[std::net::SocketAddr]) -> std::io::Result<crate::Handle> {
        let udp_socket = std::net::UdpSocket::bind(laddrs)?;

        udp_socket.set_nonblocking(true)?;

        let upd_socket = mio::net::UdpSocket::from_std(udp_socket);

        Ok((Description::UdpSocket, WithPoller::new(upd_socket)).into())
    }

    fn udp_socket_sendto(
        &self,
        waker: Waker,
        handle: crate::Handle,
        buf: &[u8],
        raddr: std::net::SocketAddr,
    ) -> std::io::Result<usize> {
        handle.expect(Description::UdpSocket)?;

        let typed_handle = TypedHandle::<WithPoller<mio::net::UdpSocket>>::new(handle);

        typed_handle.with_mut(|socket| {
            self.nonblocking_call(
                socket.get_poller()?,
                handle.token,
                Interest::Writable,
                waker,
                || socket.send_to(buf, raddr),
            )
        })
    }

    fn udp_socket_recv_from(
        &self,
        waker: Waker,
        handle: crate::Handle,
        buf: &mut [u8],
    ) -> std::io::Result<(usize, std::net::SocketAddr)> {
        handle.expect(Description::UdpSocket)?;

        let typed_handle = TypedHandle::<WithPoller<mio::net::UdpSocket>>::new(handle);

        typed_handle.with_mut(|socket| {
            self.nonblocking_call(
                socket.get_poller()?,
                handle.token,
                Interest::Readable,
                waker,
                || socket.recv_from(buf),
            )
        })
    }

    fn udp_socket_close(&self, handle: crate::Handle) -> std::io::Result<()> {
        handle.expect(Description::UdpSocket)?;

        handle.drop_as::<WithPoller<mio::net::UdpSocket>>();

        Ok(())
    }

    fn poller_open(&self, local: bool) -> std::io::Result<crate::Handle> {
        Ok((Description::Poller, MioPollerWithNotifier::new(local)).into())
    }

    fn poller_clone(&self, handle: crate::Handle) -> std::io::Result<crate::Handle> {
        handle.expect(Description::Poller)?;

        let cloned =
            TypedHandle::<MioPollerWithNotifier>::new(handle).with(|poller| poller.clone());

        Ok((Description::Poller, cloned).into())
    }

    fn poller_register(
        &self,
        poller: crate::Handle,
        source: crate::Handle,
        interests: crate::Interest,
    ) -> std::io::Result<()> {
        poller.expect(Description::Poller)?;

        TypedHandle::<MioPollerWithNotifier>::new(poller).with(|poller| {
            match source.desc {
                Description::File => todo!(),
                Description::TcpListener => {
                    let typed_handle =
                        TypedHandle::<WithPoller<mio::net::TcpListener>>::new(source);

                    typed_handle.with_mut(|with_poller| with_poller.poller = Some(poller.clone()));
                }
                Description::TcpStream => {
                    let typed_handle = TypedHandle::<WithPoller<mio::net::TcpStream>>::new(source);

                    typed_handle.with_mut(|with_poller| with_poller.poller = Some(poller.clone()));
                }
                Description::UdpSocket => {
                    let typed_handle = TypedHandle::<WithPoller<mio::net::UdpSocket>>::new(source);

                    typed_handle.with_mut(|with_poller| with_poller.poller = Some(poller.clone()));
                }
                Description::Timeout => {
                    let typed_handle = TypedHandle::<WithPoller<MioTimeout>>::new(source);

                    typed_handle.with_mut(|with_poller| with_poller.poller = Some(poller.clone()));
                }
                Description::Poller => panic!("Register poller on poller"),
                Description::External(_) => todo!(),
            }

            poller.register(source, interests)
        })
    }

    fn poller_reregister(
        &self,
        poller: crate::Handle,
        source: crate::Handle,
        interests: crate::Interest,
    ) -> std::io::Result<()> {
        poller.expect(Description::Poller)?;

        TypedHandle::<MioPollerWithNotifier>::new(poller)
            .with(|poller| poller.reregister(source, interests))
    }

    fn poller_deregister(
        &self,
        poller: crate::Handle,
        source: crate::Handle,
    ) -> std::io::Result<()> {
        poller.expect(Description::Poller)?;

        TypedHandle::<MioPollerWithNotifier>::new(poller).with(|poller| poller.deregister(source))
    }

    fn poller_poll_once(
        &self,
        poller: crate::Handle,
        timeout: Option<std::time::Duration>,
    ) -> std::io::Result<()> {
        poller.expect(Description::Poller)?;

        TypedHandle::<MioPollerWithNotifier>::new(poller).with(|poller| {
            let events = poller.poll_once(timeout)?;

            for (token, interests) in events {
                poller.on(token, interests);
            }

            Ok(())
        })
    }

    fn poller_close(&self, poller: crate::Handle) -> std::io::Result<()> {
        poller.expect(Description::Poller)?;

        poller.drop_as::<MioPollerWithNotifier>();

        Ok(())
    }

    fn tcp_listener_local_addr(&self, handle: crate::Handle) -> io::Result<std::net::SocketAddr> {
        handle.expect(Description::TcpListener)?;

        TypedHandle::<WithPoller<mio::net::TcpListener>>::new(handle)
            .with(|socket| socket.local_addr())
    }

    fn tcp_stream_local_addr(&self, handle: crate::Handle) -> io::Result<std::net::SocketAddr> {
        handle.expect(Description::TcpStream)?;

        TypedHandle::<WithPoller<mio::net::TcpStream>>::new(handle)
            .with(|socket| socket.local_addr())
    }

    fn tcp_stream_remote_addr(&self, handle: crate::Handle) -> io::Result<std::net::SocketAddr> {
        handle.expect(Description::TcpStream)?;

        TypedHandle::<WithPoller<mio::net::TcpStream>>::new(handle)
            .with(|socket| socket.peer_addr())
    }

    fn udp_local_addr(&self, handle: crate::Handle) -> io::Result<std::net::SocketAddr> {
        handle.expect(Description::UdpSocket)?;

        TypedHandle::<WithPoller<mio::net::UdpSocket>>::new(handle)
            .with(|socket| socket.local_addr())
    }
}

pub fn mio_driver() -> Driver {
    MioDriver::new().into_raw_driver().into()
}
