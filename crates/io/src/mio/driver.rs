use std::{
    io::{self, Read, Write},
    task::Waker,
    time::Duration,
};

use crate::{
    mio::{timer::MioTimer, with_poller::MioWithPoller},
    Description, Driver, Handle, Interest, IntoRawDriver, RawDriverExt, Token, TypedHandle,
};

use super::poller::MioPoller;

#[derive(Debug, Default, Clone)]
struct MioDriver {}

impl MioDriver {
    fn nonblocking_call<R, F>(
        &self,
        poller: &MioPoller,
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

                Err(err) => {
                    log::trace!("nonblocking_call error: {}", err);
                    return Err(err);
                }

                Ok(r) => {
                    _ = poller.remove_waker(token, interests);

                    return Ok(r);
                }
            }
        }
    }
}

impl RawDriverExt for MioDriver {
    #[allow(unused)]
    fn fd_user_define_open(&self, id: usize, buf: &[u8]) -> std::io::Result<crate::Handle> {
        todo!()
    }

    #[allow(unused)]
    fn fd_user_define_close(&self, id: usize, handle: crate::Handle) -> std::io::Result<()> {
        todo!()
    }

    #[allow(unused)]
    fn fd_user_define_clone(&self, handle: crate::Handle) -> std::io::Result<crate::Handle> {
        todo!()
    }

    #[allow(unused)]
    fn file_open(&self, path: &str, mode: crate::FileMode) -> std::io::Result<crate::Handle> {
        todo!()
    }

    #[allow(unused)]
    fn file_write(
        &self,
        _waker: std::task::Waker,
        _handle: crate::Handle,
        _buf: &[u8],
    ) -> std::io::Result<usize> {
        todo!()
    }

    fn file_read(
        &self,
        _waker: std::task::Waker,
        _handle: crate::Handle,
        _buf: &mut [u8],
    ) -> std::io::Result<usize> {
        todo!()
    }

    #[allow(unused)]
    fn file_close(&self, handle: crate::Handle) -> std::io::Result<()> {
        todo!()
    }

    fn timeout_open(&self, duration: std::time::Duration) -> std::io::Result<crate::Handle> {
        assert!(!duration.is_zero(), "create timeout with zero duration");

        Ok((
            Description::Timeout,
            MioWithPoller::new(MioTimer::new(duration)),
        )
            .into())
    }

    fn timeout(&self, waker: std::task::Waker, handle: crate::Handle) -> std::io::Result<bool> {
        handle.expect(Description::Timeout)?;

        let typed_handle = TypedHandle::<MioWithPoller<MioTimer>>::new(handle);

        typed_handle.with_mut(|timer| {
            if timer.is_expired() {
                log::trace!("timer, token={:?}, expired", handle.token);
                return Ok(true);
            }

            timer
                .poller()
                .add_waker(handle.token, Interest::Readable, waker);

            Ok(false)
        })
    }

    fn timeout_close(&self, handle: crate::Handle) -> std::io::Result<()> {
        handle.expect(Description::Timeout)?;

        let typed_handle = TypedHandle::<MioWithPoller<MioTimer>>::new(handle);

        typed_handle.with_mut(|timer| timer.poller().deregister(handle))?;

        handle.drop_as::<MioWithPoller<MioTimer>>();

        Ok(())
    }

    fn tcp_listener_bind(&self, laddrs: &[std::net::SocketAddr]) -> std::io::Result<crate::Handle> {
        let tcp_listener = std::net::TcpListener::bind(laddrs)?;

        tcp_listener.set_nonblocking(true)?;

        let tcp_lisener = mio::net::TcpListener::from_std(tcp_listener);

        Ok((Description::TcpListener, MioWithPoller::new(tcp_lisener)).into())
    }

    fn tcp_listener_accept(
        &self,
        waker: std::task::Waker,
        handle: crate::Handle,
    ) -> std::io::Result<(crate::Handle, std::net::SocketAddr)> {
        handle.expect(Description::TcpListener)?;

        let typed_handle = TypedHandle::<MioWithPoller<mio::net::TcpListener>>::new(handle);

        typed_handle.with(|socket| {
            self.nonblocking_call(
                socket.poller(),
                handle.token,
                Interest::Readable,
                waker,
                || socket.accept(),
            )
            .map(|(stream, raddr)| {
                (
                    (Description::TcpStream, MioWithPoller::new(stream)).into(),
                    raddr,
                )
            })
        })
    }

    fn tcp_listener_close(&self, handle: crate::Handle) -> std::io::Result<()> {
        handle.expect(Description::TcpListener)?;

        let typed_handle = TypedHandle::<MioWithPoller<mio::net::TcpListener>>::new(handle);

        typed_handle.with_mut(|timer| timer.poller().deregister(handle))?;

        handle.drop_as::<MioWithPoller<mio::net::TcpListener>>();

        Ok(())
    }

    fn tcp_stream_connect(
        &self,
        raddrs: &[std::net::SocketAddr],
    ) -> std::io::Result<crate::Handle> {
        let tcp_stream = std::net::TcpStream::connect(raddrs)?;

        tcp_stream.set_nonblocking(true)?;

        let tcp_stream = mio::net::TcpStream::from_std(tcp_stream);

        Ok((Description::TcpStream, MioWithPoller::new(tcp_stream)).into())
    }

    fn tcp_stream_write(
        &self,
        waker: std::task::Waker,
        handle: crate::Handle,
        buf: &[u8],
    ) -> std::io::Result<usize> {
        handle.expect(Description::TcpStream)?;

        let typed_handle = TypedHandle::<MioWithPoller<mio::net::TcpStream>>::new(handle);

        typed_handle.with_mut(|socket| {
            self.nonblocking_call(
                &socket.poller().clone(),
                handle.token,
                Interest::Writable,
                waker,
                || socket.write(buf),
            )
        })
    }

    fn tcp_stream_read(
        &self,
        waker: std::task::Waker,
        handle: crate::Handle,
        buf: &mut [u8],
    ) -> std::io::Result<usize> {
        handle.expect(Description::TcpStream)?;

        let typed_handle = TypedHandle::<MioWithPoller<mio::net::TcpStream>>::new(handle);

        typed_handle.with_mut(|socket| {
            self.nonblocking_call(
                &socket.poller().clone(),
                handle.token,
                Interest::Readable,
                waker,
                || socket.read(buf),
            )
        })
    }

    fn tcp_stream_close(&self, handle: crate::Handle) -> std::io::Result<()> {
        handle.expect(Description::TcpStream)?;

        let typed_handle = TypedHandle::<MioWithPoller<mio::net::TcpStream>>::new(handle);

        typed_handle.with_mut(|timer| timer.poller().deregister(handle))?;

        handle.drop_as::<MioWithPoller<mio::net::TcpStream>>();

        Ok(())
    }

    fn udp_socket_bind(&self, laddrs: &[std::net::SocketAddr]) -> std::io::Result<crate::Handle> {
        let udp_socket = std::net::UdpSocket::bind(laddrs)?;

        udp_socket.set_nonblocking(true)?;

        let upd_socket = mio::net::UdpSocket::from_std(udp_socket);

        Ok((Description::UdpSocket, MioWithPoller::new(upd_socket)).into())
    }

    fn udp_socket_sendto(
        &self,
        waker: std::task::Waker,
        handle: crate::Handle,
        buf: &[u8],
        raddr: std::net::SocketAddr,
    ) -> std::io::Result<usize> {
        handle.expect(Description::UdpSocket)?;

        let typed_handle = TypedHandle::<MioWithPoller<mio::net::UdpSocket>>::new(handle);

        typed_handle.with_mut(|socket| {
            self.nonblocking_call(
                socket.poller(),
                handle.token,
                Interest::Writable,
                waker,
                || socket.send_to(buf, raddr),
            )
        })
    }

    fn udp_socket_recv_from(
        &self,
        waker: std::task::Waker,
        handle: crate::Handle,
        buf: &mut [u8],
    ) -> std::io::Result<(usize, std::net::SocketAddr)> {
        handle.expect(Description::UdpSocket)?;

        let typed_handle = TypedHandle::<MioWithPoller<mio::net::UdpSocket>>::new(handle);

        typed_handle.with_mut(|socket| {
            self.nonblocking_call(
                socket.poller(),
                handle.token,
                Interest::Readable,
                waker,
                || socket.recv_from(buf),
            )
        })
    }

    fn udp_socket_close(&self, handle: crate::Handle) -> std::io::Result<()> {
        handle.expect(Description::UdpSocket)?;

        let typed_handle = TypedHandle::<MioWithPoller<mio::net::UdpSocket>>::new(handle);

        typed_handle.with_mut(|timer| timer.poller().deregister(handle))?;

        handle.drop_as::<MioWithPoller<mio::net::UdpSocket>>();

        Ok(())
    }

    fn poller_open(&self, _local: bool) -> std::io::Result<crate::Handle> {
        Ok((
            Description::Poller,
            MioPoller::new(Duration::from_millis(10))?,
        )
            .into())
    }

    fn poller_clone(&self, handle: crate::Handle) -> std::io::Result<crate::Handle> {
        handle.expect(Description::Poller)?;

        let cloned = TypedHandle::<MioPoller>::new(handle).with(|poller| poller.clone());

        Ok((Description::Poller, cloned).into())
    }

    fn poller_register(
        &self,
        poller: crate::Handle,
        source: crate::Handle,
        interests: crate::Interest,
    ) -> std::io::Result<()> {
        poller.expect(Description::Poller)?;

        TypedHandle::<MioPoller>::new(poller).with(|poller| poller.register(source, interests))
    }

    fn poller_reregister(
        &self,
        _poller: crate::Handle,
        _source: crate::Handle,
        _interests: crate::Interest,
    ) -> std::io::Result<()> {
        todo!()
    }

    fn poller_deregister(
        &self,
        poller: crate::Handle,
        source: crate::Handle,
    ) -> std::io::Result<()> {
        poller.expect(Description::Poller)?;

        TypedHandle::<MioPoller>::new(poller).with(|poller| poller.deregister(source))
    }

    fn poller_poll_once(
        &self,
        poller: crate::Handle,
        duration: Option<std::time::Duration>,
    ) -> std::io::Result<()> {
        poller.expect(Description::Poller)?;

        TypedHandle::<MioPoller>::new(poller).with(|poller| poller.poll_once(duration))
    }

    fn poller_close(&self, poller: crate::Handle) -> std::io::Result<()> {
        poller.expect(Description::Poller)?;

        poller.drop_as::<MioPoller>();

        Ok(())
    }

    fn tcp_listener_local_addr(&self, handle: crate::Handle) -> io::Result<std::net::SocketAddr> {
        handle.expect(Description::TcpListener)?;

        TypedHandle::<MioWithPoller<mio::net::TcpListener>>::new(handle)
            .with(|socket| socket.local_addr())
    }

    fn tcp_stream_local_addr(&self, handle: crate::Handle) -> io::Result<std::net::SocketAddr> {
        handle.expect(Description::TcpStream)?;

        TypedHandle::<MioWithPoller<mio::net::TcpStream>>::new(handle)
            .with(|socket| socket.local_addr())
    }

    fn tcp_stream_remote_addr(&self, handle: crate::Handle) -> io::Result<std::net::SocketAddr> {
        handle.expect(Description::TcpStream)?;

        TypedHandle::<MioWithPoller<mio::net::TcpStream>>::new(handle)
            .with(|socket| socket.peer_addr())
    }

    fn udp_local_addr(&self, handle: crate::Handle) -> io::Result<std::net::SocketAddr> {
        handle.expect(Description::UdpSocket)?;

        TypedHandle::<MioWithPoller<mio::net::UdpSocket>>::new(handle)
            .with(|socket| socket.local_addr())
    }

    fn tcp_stream_shutdown(&self, handle: Handle, how: std::net::Shutdown) -> io::Result<()> {
        handle.expect(Description::TcpStream)?;

        TypedHandle::<MioWithPoller<mio::net::TcpStream>>::new(handle)
            .with(|socket| socket.shutdown(how))
    }
}

pub fn mio_driver() -> Driver {
    MioDriver::default().into_raw_driver().into()
}
