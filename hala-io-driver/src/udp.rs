use std::{io, net::SocketAddr, ptr::NonNull};

/// The `RawUdpSocket` object allow the framework to generate [`UdpSocket`] instance.
pub trait RawUdpSocket {
    /// Sends data on the socket to the given address. On success, returns the
    /// number of bytes written.
    fn send_to(&self, buf: &[u8], raddr: SocketAddr) -> io::Result<usize>;

    /// Receives data from the socket. On success, returns the number of bytes
    /// read and the address from whence the data came.
    fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)>;

    /// Returns cloned `Self` if the implementation supports this.
    ///
    /// Both handles will read and write the same socket of data.
    fn try_clone(&self) -> io::Result<Self>
    where
        Self: Sized;
}

#[repr(C)]
struct RawUdpSocketVTable {
    send_to: unsafe fn(NonNull<RawUdpSocketVTable>, &[u8], SocketAddr) -> io::Result<usize>,
    recv_from: unsafe fn(NonNull<RawUdpSocketVTable>, &mut [u8]) -> io::Result<(usize, SocketAddr)>,
    try_clone: unsafe fn(NonNull<RawUdpSocketVTable>) -> io::Result<UdpSocket>,
}

impl RawUdpSocketVTable {
    fn new<R: RawUdpSocket>() -> Self {
        unsafe fn recv_from<R: RawUdpSocket>(
            ptr: NonNull<RawUdpSocketVTable>,
            buf: &mut [u8],
        ) -> io::Result<(usize, SocketAddr)> {
            let header = ptr.cast::<RawUdpSocketHeader<R>>();

            header
                .as_ref()
                .raw_udpsocket
                .recv_from(buf)
                .map(|p| p.into())
        }

        unsafe fn send_to<R: RawUdpSocket>(
            ptr: NonNull<RawUdpSocketVTable>,
            buf: &[u8],
            raddr: SocketAddr,
        ) -> io::Result<usize> {
            let header = ptr.cast::<RawUdpSocketHeader<R>>();

            header
                .as_ref()
                .raw_udpsocket
                .send_to(buf, raddr)
                .map(|p| p.into())
        }

        unsafe fn try_clone<R: RawUdpSocket>(
            ptr: NonNull<RawUdpSocketVTable>,
        ) -> io::Result<UdpSocket> {
            let header = ptr.cast::<RawUdpSocketHeader<R>>();

            header.as_ref().raw_udpsocket.try_clone().map(|p| p.into())
        }

        Self {
            send_to: send_to::<R>,
            recv_from: recv_from::<R>,
            try_clone: try_clone::<R>,
        }
    }
}

#[repr(C)]
struct RawUdpSocketHeader<R: RawUdpSocket> {
    vtable: RawUdpSocketVTable,
    raw_udpsocket: R,
}

/// The udp socket type providing by driver framework
pub struct UdpSocket {
    ptr: NonNull<RawUdpSocketVTable>,
}

impl UdpSocket {
    /// Create `UdpSocket` from [`RawUdpSocket`]
    pub fn new<R: RawUdpSocket>(raw_udpsocket: R) -> Self {
        let boxed = Box::new(RawUdpSocketHeader::<R> {
            vtable: RawUdpSocketVTable::new::<R>(),
            raw_udpsocket,
        });

        let ptr =
            unsafe { NonNull::new_unchecked(Box::into_raw(boxed) as *mut RawUdpSocketVTable) };

        Self { ptr }
    }

    /// Receives data from the socket. On success, returns the number of bytes
    /// read and the address from whence the data came.
    pub fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        unsafe { (self.ptr.as_ref().recv_from)(self.ptr, buf) }
    }

    /// Sends data on the socket to the given address. On success, returns the
    /// number of bytes written.
    pub fn send_to(&self, buf: &mut [u8], raddr: SocketAddr) -> io::Result<usize> {
        unsafe { (self.ptr.as_ref().send_to)(self.ptr, buf, raddr) }
    }

    /// Returns cloned `Self` if the implementation supports this
    pub fn try_clone(&self) -> io::Result<Self> {
        unsafe { (self.ptr.as_ref().try_clone)(self.ptr) }
    }
}

impl<T: RawUdpSocket> From<T> for UdpSocket {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}
