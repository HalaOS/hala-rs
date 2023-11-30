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
    None,
    /// When opening a file system file entry, the implementation uses this varaint as the path to the open file.
    OpenFile(&'a str),

    /// TcpListener use this varaint to bound local addresses.
    Bind(&'a [SocketAddr]),

    /// UdpSocket / TcpStream use this variant to connect remote peer
    Connect(&'a [SocketAddr]),

    /// The ops to open tick file description.
    Tick {
        duration: Duration,
        oneshot: bool,
    },

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
    None,
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

    Tick(usize),

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

    pub fn try_into_readiness(self) -> io::Result<Vec<Event>> {
        match self {
            Self::Readiness(events) => Ok(events),
            v => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Can't convert 'CtlOps' to Readiness, {:?}", v),
            )),
        }
    }

    pub fn try_into_tick(self) -> io::Result<usize> {
        match self {
            Self::Tick(times) => Ok(times),
            v => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Can't convert 'CtlOps' to Readiness, {:?}", v),
            )),
        }
    }
}

/// User defined driver must implement this trait
pub trait RawDriver {
    /// Open file description
    fn fd_open(&self, desc: Description, ops: OpenOps) -> io::Result<Handle>;

    /// Close file description by `handle`.
    fn fd_close(&self, handle: Handle) -> io::Result<()>;

    fn fd_read(&self, handle: Handle, buf: &mut [u8]) -> io::Result<ReadOps>;

    fn fd_write(&self, handle: Handle, ops: WriteOps) -> io::Result<usize>;

    fn fd_ctl(&self, handle: Handle, ops: CtlOps) -> io::Result<CtlOps>;

    fn fd_clone(&self, handle: Handle) -> io::Result<Handle>;

    fn try_clone_boxed(&self) -> io::Result<Box<dyn RawDriver + Sync + Send>>;
}

/// reactor io driver
pub struct Driver {
    inner: Box<dyn RawDriver + Send>,
}

impl Driver {
    pub fn new<R: RawDriver + Send + 'static>(raw: R) -> Self {
        Self {
            inner: Box::new(raw),
        }
    }

    #[inline]
    pub fn fd_open(&self, desc: Description, ops: OpenOps) -> io::Result<Handle> {
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

    #[inline]
    pub fn fd_clone(&self, handle: Handle) -> io::Result<Handle> {
        self.inner.fd_clone(handle)
    }
}

impl<R: RawDriver + Sync + Send + 'static> From<R> for Driver {
    fn from(value: R) -> Self {
        Self::new(value)
    }
}

impl Clone for Driver {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.try_clone_boxed().unwrap(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread::spawn;

    use super::*;

    struct MockDriver {}

    #[allow(unused)]
    impl RawDriver for MockDriver {
        fn fd_open(&self, desc: Description, ops: OpenOps) -> io::Result<Handle> {
            todo!()
        }

        fn fd_close(&self, handle: Handle) -> io::Result<()> {
            todo!()
        }

        fn fd_read(&self, handle: Handle, buf: &mut [u8]) -> io::Result<ReadOps> {
            todo!()
        }

        fn fd_write(&self, handle: Handle, ops: WriteOps) -> io::Result<usize> {
            todo!()
        }

        fn fd_ctl(&self, handle: Handle, ops: CtlOps) -> io::Result<CtlOps> {
            todo!()
        }

        fn try_clone_boxed(&self) -> io::Result<Box<dyn RawDriver + Sync + Send>> {
            todo!()
        }

        fn fd_clone(&self, handle: Handle) -> io::Result<Handle> {
            todo!()
        }
    }

    #[test]
    fn trait_send_test() {
        let driver = Driver::from(MockDriver {});

        spawn(move || driver.fd_open(Description::File, OpenOps::OpenFile("test.text")));
    }
}
