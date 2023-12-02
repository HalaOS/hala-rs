use std::io;
use std::task::Waker;
use std::time::Duration;

use std::net::SocketAddr;

use crate::{CmdResp, Description, FileMode, Handle, Interest, IntoRawDriver, RawDriver};

/// Easier to implement version of `RawDriver` trait
pub trait RawDriverExt {
    fn fd_user_define_open(&self, id: usize, buf: &[u8]) -> io::Result<Handle>;

    fn fd_user_define_close(&self, id: usize, handle: Handle) -> io::Result<()>;
    fn fd_user_define_clone(&self, handle: Handle) -> io::Result<Handle>;
    /// Create new file
    fn file_open(&self, path: &str, mode: FileMode) -> io::Result<Handle>;

    fn file_write(&self, waker: Waker, handle: Handle, buf: &[u8]) -> io::Result<usize>;

    fn file_read(&self, waker: Waker, handle: Handle, buf: &mut [u8]) -> io::Result<usize>;

    /// Close file handle
    fn file_close(&self, handle: Handle) -> io::Result<()>;

    fn tick_open(&self, duration: Duration) -> io::Result<Handle>;

    fn tick_next(&self, handle: Handle, current: usize) -> io::Result<usize>;

    fn tick_close(&self, handle: Handle) -> io::Result<()>;

    /// Create new `TcpListener` socket and bound to `laddrs`
    fn tcp_listener_bind(&self, laddrs: &[SocketAddr]) -> io::Result<Handle>;

    /// Accept one incoming `TcpStream` socket, may returns WOULD_BLOCK
    fn tcp_listener_accept(&self, waker: Waker, handle: Handle)
        -> io::Result<(Handle, SocketAddr)>;

    /// Close `TcpListener` socket.
    fn tcp_listener_close(&self, handle: Handle) -> io::Result<()>;

    /// Create new `TcpStream` socket and try connect to remote peer.
    fn tcp_stream_connect(&self, raddrs: &[SocketAddr]) -> io::Result<Handle>;

    /// Write data to underly `TcpStream`
    fn tcp_stream_write(&self, waker: Waker, handle: Handle, buf: &[u8]) -> io::Result<usize>;

    /// Read data from underly `TcpStream`
    fn tcp_stream_read(&self, waker: Waker, handle: Handle, buf: &mut [u8]) -> io::Result<usize>;

    /// Close `TcpStream` socket.
    fn tcp_stream_close(&self, handle: Handle) -> io::Result<()>;

    /// Create a new `UdpSocket` and bind to `laddrs`
    fn udp_socket_bind(&self, laddrs: &[SocketAddr]) -> io::Result<Handle>;

    /// Send one datagram to `raddr` peer
    fn udp_socket_sendto(
        &self,
        waker: Waker,
        handle: Handle,
        buf: &[u8],
        raddr: SocketAddr,
    ) -> io::Result<usize>;

    /// Recv one datagram from peer.
    fn udp_socket_recv_from(
        &self,
        waker: Waker,
        handle: Handle,
        buf: &mut [u8],
    ) -> io::Result<(usize, SocketAddr)>;

    /// Close `UdpSocket`
    fn udp_socket_close(&self, handle: Handle) -> io::Result<()>;

    /// Create new readiness io event poller.
    fn poller_open(&self) -> io::Result<Handle>;

    /// Clone pller handle.
    fn poller_clone(&self, handle: Handle) -> io::Result<Handle>;

    /// Register interests events of one source.
    fn poller_register(
        &self,
        poller: Handle,
        source: Handle,
        interests: Interest,
    ) -> io::Result<()>;

    /// Re-register interests events of one source.
    fn poller_reregister(
        &self,
        poller: Handle,
        source: Handle,
        interests: Interest,
    ) -> io::Result<()>;

    /// Deregister interests events of one source.
    fn poller_deregister(&self, poller: Handle, source: Handle) -> io::Result<()>;

    fn poller_poll_once(&self, handle: Handle, duration: Option<Duration>) -> io::Result<()>;

    /// Close poller
    fn poller_close(&self, handle: Handle) -> io::Result<()>;
}

/// Adapter `RawDriverExt` trait to `RawDriver` trait
#[derive(Clone)]
pub struct RawDriverExtProxy<T: Clone> {
    inner: T,
}

impl<T: RawDriverExt + Clone> RawDriverExtProxy<T> {
    pub fn new(inner: T) -> Self {
        Self { inner }
    }
}

impl<T: RawDriverExt + Clone> RawDriver for RawDriverExtProxy<T> {
    fn fd_open(
        &self,
        desc: crate::Description,
        open_flags: crate::OpenFlags,
    ) -> io::Result<Handle> {
        match desc {
            crate::Description::File => {
                let (path, mode) = open_flags.try_into_open_file()?;

                self.inner.file_open(path, mode)
            }
            crate::Description::TcpListener => {
                let laddrs = open_flags.try_into_bind()?;

                self.inner.tcp_listener_bind(laddrs)
            }
            crate::Description::TcpStream => {
                let laddrs = open_flags.try_into_connect()?;

                self.inner.tcp_stream_connect(laddrs)
            }
            crate::Description::UdpSocket => {
                let laddrs = open_flags.try_into_bind()?;

                self.inner.udp_socket_bind(laddrs)
            }
            crate::Description::Tick => {
                let duration = open_flags.try_into_duration()?;

                self.inner.tick_open(duration)
            }
            crate::Description::Poller => self.inner.poller_open(),
            crate::Description::External(id) => {
                let buf = open_flags.try_into_user_defined()?;

                self.inner.fd_user_define_open(id, buf)
            }
        }
    }

    fn fd_cntl(&self, handle: Handle, cmd: crate::Cmd) -> io::Result<crate::CmdResp> {
        match cmd {
            crate::Cmd::Read { waker, buf } => match handle.desc {
                Description::File => self
                    .inner
                    .file_read(waker, handle, buf)
                    .map(|len| CmdResp::ReadData(len)),
                Description::TcpStream => self
                    .inner
                    .tcp_stream_read(waker, handle, buf)
                    .map(|len| CmdResp::ReadData(len)),
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("Expect File / TcpStream , but got {:?}", handle.desc),
                    ));
                }
            },
            crate::Cmd::Write { waker, buf } => match handle.desc {
                Description::File => self
                    .inner
                    .file_write(waker, handle, buf)
                    .map(|len| CmdResp::WriteData(len)),
                Description::TcpStream => self
                    .inner
                    .tcp_stream_write(waker, handle, buf)
                    .map(|len| CmdResp::WriteData(len)),
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("Expect File / TcpStream , but got {:?}", handle.desc),
                    ));
                }
            },
            crate::Cmd::SendTo { waker, buf, raddr } => {
                handle.expect(Description::UdpSocket)?;

                self.inner
                    .udp_socket_sendto(waker, handle, buf, raddr)
                    .map(|len| CmdResp::WriteData(len))
            }
            crate::Cmd::RecvFrom { waker, buf } => {
                handle.expect(Description::UdpSocket)?;

                self.inner
                    .udp_socket_recv_from(waker, handle, buf)
                    .map(|(len, raddr)| CmdResp::RecvFrom(len, raddr))
            }
            crate::Cmd::Register { source, interests } => {
                handle.expect(Description::Poller)?;

                self.inner
                    .poller_register(handle, source, interests)
                    .map(|_| CmdResp::None)
            }
            crate::Cmd::ReRegister { source, interests } => {
                handle.expect(Description::Poller)?;

                self.inner
                    .poller_reregister(handle, source, interests)
                    .map(|_| CmdResp::None)
            }
            crate::Cmd::Deregister(source) => {
                handle.expect(Description::Poller)?;

                self.inner
                    .poller_deregister(handle, source)
                    .map(|_| CmdResp::None)
            }
            crate::Cmd::Accept(waker) => {
                handle.expect(Description::TcpListener)?;

                self.inner
                    .tcp_listener_accept(waker, handle)
                    .map(|(stream, raddr)| CmdResp::Incoming(stream, raddr))
            }
            crate::Cmd::PollOnce(duration) => {
                handle.expect(Description::Poller)?;

                self.inner
                    .poller_poll_once(handle, duration)
                    .map(|_| CmdResp::None)
            }
            crate::Cmd::TryClone => match handle.desc {
                Description::Poller => self
                    .inner
                    .poller_clone(handle)
                    .map(|handle| CmdResp::Cloned(handle)),
                _ => self
                    .inner
                    .fd_user_define_clone(handle)
                    .map(|handle| CmdResp::Cloned(handle)),
            },
            crate::Cmd::Tick(current) => {
                handle.expect(Description::Tick)?;

                self.inner
                    .tick_next(handle, current)
                    .map(|next| CmdResp::Tick(next))
            }
        }
    }

    fn fd_close(&self, handle: Handle) -> io::Result<()> {
        match handle.desc {
            Description::File => self.inner.file_close(handle),
            Description::TcpListener => self.inner.tcp_listener_close(handle),
            Description::TcpStream => self.inner.tcp_stream_close(handle),
            Description::UdpSocket => self.inner.udp_socket_close(handle),
            Description::Tick => self.inner.tick_close(handle),
            Description::Poller => self.inner.poller_close(handle),
            Description::External(id) => self.inner.fd_user_define_close(id, handle),
        }
    }
}

impl<T: RawDriverExt + Clone> IntoRawDriver for T {
    type Driver = RawDriverExtProxy<T>;

    fn into_raw_driver(self) -> Self::Driver {
        RawDriverExtProxy::new(self)
    }
}
