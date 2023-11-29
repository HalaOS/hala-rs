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
#[derive(Debug, Clone)]
pub struct Event {
    pub source: Handle,
    pub interests: Interest,
    pub interests_use_define: Option<usize>,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum OpenOps<'a> {
    /// When opening a file system file entry, the implementation uses this varaint as the path to the open file.
    OpenFile(&'a str),

    /// TcpListener use this varaint to bound local addresses.
    Bind(&'a [SocketAddr]),

    /// UdpSocket / TcpStream use this variant to connect remote peer
    Connect(&'a [SocketAddr]),

    /// This value is used by the fd_open implementation when the parameter
    /// Description is `Timeout` or `Poller`.
    Timeout(Duration),

    /// The ops to open tick file description.
    Tick(Duration),

    /// The meaning of this variant is defined by the implementation
    UserDefine {
        id: usize,
        write_buf: &'a [u8],
        read_buf: &'a [u8],
    },
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum Description {
    Timeout,
    Tick,
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
    pub id: usize,
    pub desc: Description,
    data: *const (),
}

unsafe impl Send for Handle {}

unsafe impl Sync for Handle {}

impl Default for Handle {
    fn default() -> Self {
        Self {
            id: 0,
            desc: Description::TcpStream,
            data: null(),
        }
    }
}

impl Handle {
    /// Create new handle instance.
    pub fn new(id: usize, desc: Description, data: *const ()) -> Self {
        Self { id, desc, data }
    }
}

#[derive(Debug)]
pub enum ReadOps {
    Read(usize),

    RecvFrom(usize, SocketAddr),
}

impl ReadOps {
    pub fn try_to_read(self) -> io::Result<usize> {
        match self {
            Self::Read(len) => Ok(len),
            v => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Can't convert 'ReadOps' to Read, {:?}", v),
            )),
        }
    }

    pub fn try_to_recv_from(self) -> io::Result<(usize, SocketAddr)> {
        match self {
            Self::RecvFrom(len, raddr) => Ok((len, raddr)),
            v => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Can't convert 'ReadOps' to RecvFrom, {:?}", v),
            )),
        }
    }
}

pub enum WriteOps<'a> {
    Write(&'a [u8]),

    SendTo(&'a [u8], SocketAddr),
}

/// fd_ctl method operation/result variants.
#[derive(Debug)]
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

    /// Poll for a ready event once, if there is no ready event will wait for `Duration`
    PollOnce(Option<Duration>),

    /// Readiness io events collection, this variant usually returns by `PollOnce` method.
    Readiness(Vec<Event>),

    OpenFile(&'a str),

    Bind(&'a [SocketAddr]),
    Accept,
    Incoming(Handle, SocketAddr),
    Connect(&'a [SocketAddr]),
    UserDefine {
        id: usize,
        write_buf: &'a [u8],
        read_buf: &'a [u8],
    },
}

impl<'a> CtlOps<'a> {
    pub fn try_into_incoming(self) -> io::Result<(Handle, SocketAddr)> {
        match self {
            Self::Incoming(handle, raddr) => Ok((handle, raddr)),
            v => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Can't convert 'CtlOps' to Incoming, {:?}", v),
            )),
        }
    }
}

/// User defined driver must implement this trait
pub trait RawDriver {
    /// Open file description
    fn fd_open(&self, desc: Description, ops: Option<OpenOps>) -> io::Result<Handle>;

    /// Close file description by `handle`.
    fn fd_close(&self, handle: Handle) -> io::Result<()>;

    fn fd_read(&self, handle: Handle, buf: &mut [u8]) -> io::Result<ReadOps>;

    fn fd_write(&self, handle: Handle, ops: WriteOps) -> io::Result<usize>;

    fn fd_ctl(&self, handle: Handle, ops: CtlOps) -> io::Result<CtlOps>;

    fn try_clone_boxed(&self) -> io::Result<Box<dyn RawDriver + Sync + Send>>;
}

/// reactor io driver
pub struct Driver {
    inner: Box<dyn RawDriver + Sync + Send>,
}

impl Driver {
    #[inline]
    pub fn fd_open(&self, desc: Description, ops: Option<OpenOps>) -> io::Result<Handle> {
        self.inner.fd_open(desc, ops)
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

    #[inline]
    pub fn fd_ctl(&self, handle: Handle, ops: CtlOps) -> io::Result<CtlOps> {
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
