use std::{
    io,
    net::{Shutdown, SocketAddr},
    ptr::NonNull,
    task::Waker,
    time::Duration,
};

use bitmask_enum::bitmask;

use crate::{Description, Handle, Interest};

#[bitmask]
pub enum FileMode {
    Read,
    Write,
    Create,
    Truncate,
}

/// File description open flags used by `fd_open` method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OpenFlags<'a> {
    None,
    /// The path to the local file system to open.
    OpenFile(&'a str, FileMode),
    /// The binding addrs of the opening socket
    Bind(&'a [SocketAddr]),
    /// The address list of the remote peer to which the open socket will connect
    Connect(&'a [SocketAddr]),
    Duration(Duration),
    UserDefined(&'a [u8]),
    /// Flag to create poller in single thread mode.
    LocalPoller,
}

impl<'a> OpenFlags<'a> {
    pub fn try_into_open_file(self) -> io::Result<(&'a str, FileMode)> {
        match self {
            Self::OpenFile(path, mode) => Ok((path, mode)),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Expect OpenFile, but got {:?}", self),
            )),
        }
    }

    pub fn try_into_bind(self) -> io::Result<&'a [SocketAddr]> {
        match self {
            Self::Bind(laddrs) => Ok(laddrs),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Expect Bind, but got {:?}", self),
            )),
        }
    }

    pub fn try_into_connect(self) -> io::Result<&'a [SocketAddr]> {
        match self {
            Self::Connect(raddrs) => Ok(raddrs),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Expect Bind, but got {:?}", self),
            )),
        }
    }

    pub fn try_into_duration(self) -> io::Result<Duration> {
        match self {
            Self::Duration(duration) => Ok(duration),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Expect Bind, but got {:?}", self),
            )),
        }
    }

    pub fn try_into_user_defined(self) -> io::Result<&'a [u8]> {
        match self {
            Self::UserDefined(buf) => Ok(buf),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Expect UserDefined, but got {:?}", self),
            )),
        }
    }
}

/// File description control command.
#[derive(Debug)]
pub enum Cmd<'a> {
    /// Write data to stream file description.
    Read {
        waker: Waker,
        buf: &'a mut [u8],
    },
    /// Read data from stream file description.
    Write {
        waker: Waker,
        buf: &'a [u8],
    },

    /// Command `Sendto` parameter for udp socket.
    SendTo {
        waker: Waker,
        buf: &'a [u8],
        raddr: SocketAddr,
    },

    /// Command to invoke UdpSocket `recv_from` method.
    RecvFrom {
        waker: Waker,
        buf: &'a mut [u8],
    },

    /// Register io event interests with `Poll`
    Register {
        source: Handle,
        interests: Interest,
    },

    /// Re-register io event interests with `Poll`
    ReRegister {
        source: Handle,
        interests: Interest,
    },

    /// Deregister io event interests with `Poll`
    // Deregister(Handle),

    /// Try accept one incoming connection.
    Accept(Waker),

    /// Poll once io readiness events.
    PollOnce(Option<Duration>),

    /// Try to clone the handle.
    TryClone,
    Timeout(Waker),
    LocalAddr,
    RemoteAddr,

    Shutdown(Shutdown),
}

/// The response of `fd_cntl` .
#[derive(Debug, Clone)]
pub enum CmdResp {
    None,
    /// Command `RecvFrom` response data.
    RecvFrom(usize, SocketAddr),
    /// Command `Accept` response data.
    Incoming(Handle, SocketAddr),
    /// Command `Write` / `SendTo` response data
    DataLen(usize),
    Timeout(bool),
    /// Command `TryClone` response data.
    Cloned(Handle),
    SockAddr(SocketAddr),
}

impl CmdResp {
    pub fn try_into_incoming(self) -> io::Result<(Handle, SocketAddr)> {
        match self {
            Self::Incoming(handle, raddr) => Ok((handle, raddr)),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Expect Incoming, but got {:?}", self),
            )),
        }
    }

    pub fn try_into_recv_from(self) -> io::Result<(usize, SocketAddr)> {
        match self {
            Self::RecvFrom(len, raddr) => Ok((len, raddr)),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Expect Incoming, but got {:?}", self),
            )),
        }
    }

    pub fn try_into_sockaddr(self) -> io::Result<SocketAddr> {
        match self {
            Self::SockAddr(addr) => Ok(addr),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Expect SockAddr, but got {:?}", self),
            )),
        }
    }

    pub fn try_into_datalen(self) -> io::Result<usize> {
        match self {
            Self::DataLen(len) => Ok(len),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Expect SockAddr, but got {:?}", self),
            )),
        }
    }

    pub fn try_into_timeout(self) -> io::Result<bool> {
        match self {
            Self::Timeout(status) => Ok(status),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Expect Timeout, but got {:?}", self),
            )),
        }
    }
}

/// io driver must implement this trait.
pub trait RawDriver {
    /// Try open file description.
    fn fd_open(&self, desc: Description, open_flags: OpenFlags) -> io::Result<Handle>;

    /// performs one of file description operation.
    fn fd_cntl(&self, handle: Handle, cmd: Cmd) -> io::Result<CmdResp>;

    /// Close the opened file description.
    ///
    /// #Panic
    ///
    /// Closing the `Handle` twice must cause panic
    fn fd_close(&self, handle: Handle) -> io::Result<()>;
}

#[repr(C)]
#[derive(Clone)]
struct DriverVTable {
    fd_open: unsafe fn(NonNull<DriverVTable>, Description, OpenFlags) -> io::Result<Handle>,
    fd_cntl: unsafe fn(NonNull<DriverVTable>, Handle, Cmd) -> io::Result<CmdResp>,
    fd_close: unsafe fn(NonNull<DriverVTable>, Handle) -> io::Result<()>,
    clone: unsafe fn(NonNull<DriverVTable>) -> Driver,
    drop: unsafe fn(NonNull<DriverVTable>),
}

impl DriverVTable {
    fn new<R: RawDriver + Clone>() -> Self {
        fn fd_open<R: RawDriver + Clone>(
            ptr: NonNull<DriverVTable>,
            desc: Description,
            flags: OpenFlags,
        ) -> io::Result<Handle> {
            let header = ptr.cast::<DriverHeader<R>>();

            unsafe { header.as_ref().data.fd_open(desc, flags) }
        }

        fn fd_cntl<R: RawDriver + Clone>(
            ptr: NonNull<DriverVTable>,
            handle: Handle,
            cmd: Cmd,
        ) -> io::Result<CmdResp> {
            let header = ptr.cast::<DriverHeader<R>>();

            let result = unsafe { header.as_ref().data.fd_cntl(handle, cmd) };

            // log::trace!("[HalaIO] fd_cntl({:?}), {:?}", handle, result);

            result
        }

        fn fd_close<R: RawDriver + Clone>(
            ptr: NonNull<DriverVTable>,
            handle: Handle,
        ) -> io::Result<()> {
            let header = ptr.cast::<DriverHeader<R>>();

            unsafe { header.as_ref().data.fd_close(handle) }
        }

        fn clone<R: RawDriver + Clone>(ptr: NonNull<DriverVTable>) -> Driver {
            let driver = unsafe { ptr.cast::<DriverHeader<R>>().as_ref().clone() };

            let ptr = unsafe {
                NonNull::new_unchecked(Box::into_raw(Box::new(driver)) as *mut DriverVTable)
            };

            Driver { ptr }
        }

        fn drop<R: RawDriver + Clone>(ptr: NonNull<DriverVTable>) {
            _ = unsafe { Box::from_raw(ptr.cast::<DriverHeader<R>>().as_ptr()) };
        }

        Self {
            fd_open: fd_open::<R>,
            fd_cntl: fd_cntl::<R>,
            fd_close: fd_close::<R>,
            clone: clone::<R>,
            drop: drop::<R>,
        }
    }
}

pub trait IntoRawDriver {
    type Driver: RawDriver + Clone;
    fn into_raw_driver(self) -> Self::Driver;
}

#[repr(C)]
#[derive(Clone)]
struct DriverHeader<R: RawDriver + Clone> {
    vtable: DriverVTable,
    data: R,
}

/// The driver to driving Asynchronous IO
#[derive(Debug)]
pub struct Driver {
    ptr: NonNull<DriverVTable>,
}

unsafe impl Send for Driver {}
unsafe impl Sync for Driver {}

impl<R: RawDriver + Clone> From<R> for Driver {
    fn from(value: R) -> Self {
        Self::new(value)
    }
}

impl Driver {
    /// Creates a new `Driver` from [`RawDriver`].
    pub fn new<R: RawDriver + Clone>(raw: R) -> Self {
        let boxed = Box::new(DriverHeader::<R> {
            data: raw,
            vtable: DriverVTable::new::<R>(),
        });

        let ptr = unsafe { NonNull::new_unchecked(Box::into_raw(boxed) as *mut DriverVTable) };

        Self { ptr }
    }

    /// Try open file description.
    pub fn fd_open(&self, desc: Description, open_flags: OpenFlags) -> io::Result<Handle> {
        unsafe { (self.ptr.as_ref().fd_open)(self.ptr, desc, open_flags) }
    }

    /// performs one of file description operation.
    pub fn fd_cntl(&self, handle: Handle, cmd: Cmd) -> io::Result<CmdResp> {
        unsafe { (self.ptr.as_ref().fd_cntl)(self.ptr, handle, cmd) }
    }

    /// Close the opened file description.
    ///
    /// #Panic
    ///
    /// Closing the `Handle` twice must cause panic
    pub fn fd_close(&self, handle: Handle) -> io::Result<()> {
        unsafe { (self.ptr.as_ref().fd_close)(self.ptr, handle) }
    }
}

impl Clone for Driver {
    fn clone(&self) -> Self {
        unsafe { (self.ptr.as_ref().clone)(self.ptr) }
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        unsafe { (self.ptr.as_ref().drop)(self.ptr) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct MockDriver {}

    struct MockFile {}

    impl RawDriver for MockDriver {
        fn fd_open(
            &self,
            desc: crate::Description,
            _open_flags: crate::OpenFlags,
        ) -> std::io::Result<crate::Handle> {
            Ok(Handle::from((desc, MockFile {})))
        }

        fn fd_cntl(
            &self,
            _handle: crate::Handle,
            _cmd: crate::Cmd,
        ) -> std::io::Result<crate::CmdResp> {
            Ok(CmdResp::None)
        }

        fn fd_close(&self, handle: crate::Handle) -> std::io::Result<()> {
            handle.drop_as::<MockFile>();

            Ok(())
        }
    }

    #[test]
    fn test_driver_vtable() {
        let driver = Driver::new(MockDriver {});

        let driver = driver.clone();

        let handle = driver.fd_open(Description::File, OpenFlags::None).unwrap();

        driver.fd_cntl(handle, Cmd::PollOnce(None)).unwrap();

        driver.fd_close(handle).unwrap();
    }
}
