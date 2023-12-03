use std::{
    io::{self, Read, Write},
    marker::PhantomData,
    task::Waker,
    time::Duration,
};

use crate::{Description, Driver, Interest, IntoRawDriver, RawDriverExt, Token, TypedHandle};

use super::*;

pub struct BasicMioDriver<N, P>
where
    N: Clone,
    P: Clone,
{
    notifier: N,
    _marked: PhantomData<P>,
}

impl<N, P> BasicMioDriver<N, P>
where
    N: MioNotifier + Default + Clone,
    P: MioPoller + Default + Clone,
{
    pub fn new() -> Self {
        Self {
            notifier: Default::default(),
            _marked: Default::default(),
        }
    }
}

impl<N, P> Clone for BasicMioDriver<N, P>
where
    N: Clone,
    P: Clone,
{
    fn clone(&self) -> Self {
        Self {
            notifier: self.notifier.clone(),
            _marked: Default::default(),
        }
    }
}

impl<N, P> BasicMioDriver<N, P>
where
    N: MioNotifier + Clone,
    P: MioPoller + Clone,
{
    fn nonblocking_call<R, F>(
        &self,
        token: Token,
        interests: Interest,
        waker: Waker,
        mut f: F,
    ) -> io::Result<R>
    where
        F: FnMut() -> io::Result<R>,
    {
        self.notifier.add_waker(token, interests, waker);

        loop {
            match f() {
                Err(err) if err.kind() == io::ErrorKind::Interrupted => {
                    continue;
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => return Err(err),

                r => {
                    _ = self.notifier.remove_waker(token, interests);

                    return r;
                }
            }
        }
    }
}

impl<N, P> RawDriverExt for BasicMioDriver<N, P>
where
    N: MioNotifier + Clone,
    P: MioPoller + Default + Clone,
{
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

    fn tick_open(&self, _duration: std::time::Duration) -> std::io::Result<crate::Handle> {
        unimplemented!("Unimplement tick_open")
    }

    fn tick_next(&self, _handle: crate::Handle, _current: usize) -> std::io::Result<usize> {
        unimplemented!("Unimplement tick_next")
    }

    fn tick_close(&self, _handle: crate::Handle) -> std::io::Result<()> {
        unimplemented!("Unimplement tick_close")
    }

    fn tcp_listener_bind(&self, laddrs: &[std::net::SocketAddr]) -> std::io::Result<crate::Handle> {
        let tcp_listener = std::net::TcpListener::bind(laddrs)?;

        tcp_listener.set_nonblocking(true)?;

        let tcp_lisener = mio::net::TcpListener::from_std(tcp_listener);

        Ok((Description::TcpListener, tcp_lisener).into())
    }

    fn tcp_listener_accept(
        &self,
        waker: Waker,
        handle: crate::Handle,
    ) -> std::io::Result<(crate::Handle, std::net::SocketAddr)> {
        log::trace!("[MioDriver] TcpListener accept, {:?}", handle);

        handle.expect(Description::TcpListener)?;

        let typed_handle = TypedHandle::<mio::net::TcpListener>::new(handle);

        self.nonblocking_call(handle.token, Interest::Readable, waker, || {
            typed_handle.with(|socket| socket.accept())
        })
        .map(|(stream, raddr)| ((Description::TcpStream, stream).into(), raddr))
    }

    fn tcp_listener_close(&self, handle: crate::Handle) -> std::io::Result<()> {
        handle.expect(Description::TcpListener)?;

        handle.drop_as::<mio::net::TcpListener>();

        Ok(())
    }

    fn tcp_stream_connect(
        &self,
        raddrs: &[std::net::SocketAddr],
    ) -> std::io::Result<crate::Handle> {
        let tcp_stream = std::net::TcpStream::connect(raddrs)?;

        tcp_stream.set_nonblocking(true)?;

        let tcp_stream = mio::net::TcpStream::from_std(tcp_stream);

        Ok((Description::TcpStream, tcp_stream).into())
    }

    fn tcp_stream_write(
        &self,
        waker: Waker,
        handle: crate::Handle,
        buf: &[u8],
    ) -> std::io::Result<usize> {
        handle.expect(Description::TcpStream)?;

        let typed_handle = TypedHandle::<mio::net::TcpStream>::new(handle);

        self.nonblocking_call(handle.token, Interest::Writable, waker, || {
            typed_handle.with_mut(|socket| socket.write(buf))
        })
    }

    fn tcp_stream_read(
        &self,
        waker: Waker,
        handle: crate::Handle,
        buf: &mut [u8],
    ) -> std::io::Result<usize> {
        handle.expect(Description::TcpStream)?;

        let typed_handle = TypedHandle::<mio::net::TcpStream>::new(handle);

        self.nonblocking_call(handle.token, Interest::Readable, waker, || {
            typed_handle.with_mut(|socket| socket.read(buf))
        })
    }

    fn tcp_stream_close(&self, handle: crate::Handle) -> std::io::Result<()> {
        handle.expect(Description::TcpStream)?;

        handle.drop_as::<mio::net::TcpStream>();

        Ok(())
    }

    fn udp_socket_bind(&self, laddrs: &[std::net::SocketAddr]) -> std::io::Result<crate::Handle> {
        let udp_socket = std::net::UdpSocket::bind(laddrs)?;

        udp_socket.set_nonblocking(true)?;

        let upd_socket = mio::net::UdpSocket::from_std(udp_socket);

        Ok((Description::UdpSocket, upd_socket).into())
    }

    fn udp_socket_sendto(
        &self,
        waker: Waker,
        handle: crate::Handle,
        buf: &[u8],
        raddr: std::net::SocketAddr,
    ) -> std::io::Result<usize> {
        handle.expect(Description::UdpSocket)?;

        let typed_handle = TypedHandle::<mio::net::UdpSocket>::new(handle);

        self.nonblocking_call(handle.token, Interest::Writable, waker, || {
            typed_handle.with_mut(|socket| socket.send_to(buf, raddr))
        })
    }

    fn udp_socket_recv_from(
        &self,
        waker: Waker,
        handle: crate::Handle,
        buf: &mut [u8],
    ) -> std::io::Result<(usize, std::net::SocketAddr)> {
        handle.expect(Description::UdpSocket)?;

        let typed_handle = TypedHandle::<mio::net::UdpSocket>::new(handle);

        self.nonblocking_call(handle.token, Interest::Readable, waker, || {
            typed_handle.with_mut(|socket| socket.recv_from(buf))
        })
    }

    fn udp_socket_close(&self, handle: crate::Handle) -> std::io::Result<()> {
        handle.expect(Description::UdpSocket)?;

        handle.drop_as::<mio::net::UdpSocket>();

        Ok(())
    }

    fn poller_open(&self) -> std::io::Result<crate::Handle> {
        let poller = P::default();

        Ok((Description::Poller, poller).into())
    }

    fn poller_clone(&self, handle: crate::Handle) -> std::io::Result<crate::Handle> {
        handle.expect(Description::Poller)?;

        let cloned = TypedHandle::<P>::new(handle).with(|poller| poller.clone());

        Ok((Description::Poller, cloned).into())
    }

    fn poller_register(
        &self,
        poller: crate::Handle,
        source: crate::Handle,
        interests: crate::Interest,
    ) -> std::io::Result<()> {
        poller.expect(Description::Poller)?;

        TypedHandle::<P>::new(poller).with(|poller| poller.register(source, interests))
    }

    fn poller_reregister(
        &self,
        poller: crate::Handle,
        source: crate::Handle,
        interests: crate::Interest,
    ) -> std::io::Result<()> {
        poller.expect(Description::Poller)?;

        TypedHandle::<P>::new(poller).with(|poller| poller.reregister(source, interests))
    }

    fn poller_deregister(
        &self,
        poller: crate::Handle,
        source: crate::Handle,
    ) -> std::io::Result<()> {
        poller.expect(Description::Poller)?;

        TypedHandle::<P>::new(poller).with(|poller| poller.deregister(source))
    }

    fn poller_poll_once(
        &self,
        poller: crate::Handle,
        timeout: Option<std::time::Duration>,
    ) -> std::io::Result<()> {
        poller.expect(Description::Poller)?;

        TypedHandle::<P>::new(poller)
            .with(|poller| poller.poll_once(Some(timeout.unwrap_or(Duration::from_secs(1)))))
    }

    fn poller_close(&self, poller: crate::Handle) -> std::io::Result<()> {
        poller.expect(Description::Poller)?;

        poller.drop_as::<P>();

        Ok(())
    }

    fn tcp_listener_local_addr(&self, handle: crate::Handle) -> io::Result<std::net::SocketAddr> {
        handle.expect(Description::TcpListener)?;

        TypedHandle::<mio::net::TcpListener>::new(handle).with(|socket| socket.local_addr())
    }

    fn tcp_stream_local_addr(&self, handle: crate::Handle) -> io::Result<std::net::SocketAddr> {
        handle.expect(Description::TcpStream)?;

        TypedHandle::<mio::net::TcpStream>::new(handle).with(|socket| socket.local_addr())
    }

    fn tcp_stream_remote_addr(&self, handle: crate::Handle) -> io::Result<std::net::SocketAddr> {
        handle.expect(Description::TcpStream)?;

        TypedHandle::<mio::net::TcpStream>::new(handle).with(|socket| socket.peer_addr())
    }

    fn udp_local_addr(&self, handle: crate::Handle) -> io::Result<std::net::SocketAddr> {
        handle.expect(Description::UdpSocket)?;

        TypedHandle::<mio::net::UdpSocket>::new(handle).with(|socket| socket.local_addr())
    }
}

type STMioDriver = BasicMioDriver<STMioNotifier, STMioPoller>;

type MTMioDriver = BasicMioDriver<MTMioNotifier, MTMioPoller>;

pub fn local_mio_driver() -> Driver {
    STMioDriver::new().into_raw_driver().into()
}

pub fn mio_driver() -> Driver {
    MTMioDriver::new().into_raw_driver().into()
}
