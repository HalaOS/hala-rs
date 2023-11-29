use std::{
    io,
    net::SocketAddr,
    ptr::{null, NonNull},
    time::Duration,
};

use bitmask_enum::bitmask;

#[bitmask(u8)]
pub enum Interest {
    Read,
    Write,
    Close,
    UserDefine,
}

/// Io event object from driver
pub struct Event {
    pub source: Handle,
    pub interests: Interest,
    pub interests_use_define: Option<usize>,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum FileDescription {
    File,
    Poller,
    TcpListener,
    TcpStream,
    UdpSocket,
    UserDefine(NonNull<*const ()>),
}

/// File handle used by `fd_close`, `fd_ctl` ,etc..
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct Handle {
    id: usize,
    desc: FileDescription,
    data: *const (),
}

impl Default for Handle {
    fn default() -> Self {
        Self {
            id: 0,
            desc: FileDescription::TcpStream,
            data: null(),
        }
    }
}

impl Handle {
    /// Create new handle instance.
    pub fn new(id: usize, desc: FileDescription, data: *const ()) -> Self {
        Self { id, desc, data }
    }
}

pub enum ReadOps {
    ReadLen(usize),

    RecvFrom(usize, SocketAddr),
}

pub enum WriteOps<'a> {
    Write(&'a [u8]),

    SendTo(&'a [u8], SocketAddr),
}

pub enum CtlOps<'a> {
    Register {
        handles: &'a [Handle],
        interests: Interest,
    },
    Reregister {
        handles: &'a [Handle],
        interests: Interest,
    },
    Deregister(&'a [Handle]),

    UserDefine {
        write_buf: &'a [u8],
        read_buf: &'a [u8],
    },
}

/// Implementator provide [`Driver`] instance.
pub trait RawDriver {
    /// Open file description
    fn fd_open(&self, desc: FileDescription) -> io::Result<Handle>;

    /// Close file description by `handle`.
    fn fd_close(&self, handle: Handle) -> io::Result<()>;

    fn fd_read(&self, handle: Handle, buf: &mut [u8]) -> io::Result<ReadOps>;

    fn fd_write(&self, handle: Handle, ops: WriteOps) -> io::Result<usize>;

    /// Poll readiness events with `timeout` argument.
    fn poll_once(&self, poller: Handle, timeout: Option<Duration>) -> io::Result<Vec<Event>>;

    fn try_clone_boxed(&self) -> io::Result<Box<dyn RawDriver + Sync + Send>>;

    fn fd_ctl(&self, handle: Handle, ops: CtlOps) -> io::Result<usize>;
}

/// reactor io driver
pub struct Driver {
    inner: Box<dyn RawDriver + Sync + Send>,
}

impl Driver {
    #[inline]
    pub fn fd_open(&self, desc: FileDescription) -> io::Result<Handle> {
        self.inner.fd_open(desc)
    }

    #[inline]
    pub fn fd_close(&self, handle: Handle) -> io::Result<()> {
        self.inner.fd_close(handle)
    }

    #[inline]
    pub fn fd_read(&self, handle: Handle, buf: &mut [u8]) -> io::Result<ReadOps> {
        self.inner.fd_read(handle, buf)
    }

    #[inline]
    pub fn fd_write(&self, handle: Handle, ops: WriteOps) -> io::Result<usize> {
        self.inner.fd_write(handle, ops)
    }

    /// Poll readiness events with `timeout` argument.
    pub fn poll_once(&self, poller: Handle, timeout: Option<Duration>) -> io::Result<Vec<Event>> {
        self.inner.poll_once(poller, timeout)
    }

    #[inline]
    pub fn fd_ctl(&self, handle: Handle, ops: CtlOps) -> io::Result<usize> {
        self.inner.fd_ctl(handle, ops)
    }
}

impl<R: RawDriver + Sync + Send + 'static> From<R> for Driver {
    fn from(value: R) -> Self {
        Self {
            inner: Box::new(value),
        }
    }
}

impl Clone for Driver {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.try_clone_boxed().unwrap(),
        }
    }
}
