use std::{io, net::SocketAddr, ptr::NonNull};

use crate::{
    Poller, RawPoller, RawTcpListener, RawTcpStream, RawUdpSocket, TcpListener, TcpStream,
    UdpSocket,
};

/// The `RawDriver` allow the framework to generate [`Driver`] instance.
pub trait RawDriver {
    /// [`RawPoller`] type provided by the runtime implementor
    type Poller: RawPoller;

    /// [`RawUdpSocket`] type provided by the runtime implementor
    type UdpSocket: RawUdpSocket;

    /// [`RawTcpListener`] type provided by the runtime implementor
    type TcpListener: RawTcpListener;

    /// [`RawTcpStream`] type provided by the runtime implementor
    type TcpStream: RawTcpStream;

    /// Create new runtime io event `Poller` object.
    fn new_poller(&self) -> io::Result<Self::Poller>;

    /// Create new `UdpSocket` and bind it to `laddr`
    fn udp_bind(&self, laddr: SocketAddr) -> io::Result<Self::UdpSocket>;

    /// Create new [`TcpListener`] socket and listen incoming connection on `laddr`
    fn tcp_listen(&self, laddr: SocketAddr) -> io::Result<Self::TcpListener>;

    /// Create new [`TcpStream`] socket and connect to `raddr`
    fn tcp_connect(&self, raddr: SocketAddr) -> io::Result<Self::TcpStream>;
}

#[repr(C)]
struct RawDriverHeader<D: RawDriver> {
    vtable: RawDriverVTable,
    raw_driver: D,
}

struct RawDriverVTable {
    new_poller: unsafe fn(NonNull<RawDriverVTable>) -> io::Result<Poller>,
    udp_bind: unsafe fn(NonNull<RawDriverVTable>, SocketAddr) -> io::Result<UdpSocket>,
    tcp_listen: unsafe fn(NonNull<RawDriverVTable>, SocketAddr) -> io::Result<TcpListener>,
    tcp_connect: unsafe fn(NonNull<RawDriverVTable>, SocketAddr) -> io::Result<TcpStream>,
}

impl RawDriverVTable {
    fn new<C: RawDriver>() -> Self {
        unsafe fn new_poller<C: RawDriver>(ptr: NonNull<RawDriverVTable>) -> io::Result<Poller> {
            let header = ptr.cast::<RawDriverHeader<C>>();

            header.as_ref().raw_driver.new_poller().map(|p| p.into())
        }

        unsafe fn udp_bind<C: RawDriver>(
            ptr: NonNull<RawDriverVTable>,
            laddr: SocketAddr,
        ) -> io::Result<UdpSocket> {
            let header = ptr.cast::<RawDriverHeader<C>>();

            header.as_ref().raw_driver.udp_bind(laddr).map(|p| p.into())
        }

        unsafe fn tcp_listen<C: RawDriver>(
            ptr: NonNull<RawDriverVTable>,
            laddr: SocketAddr,
        ) -> io::Result<TcpListener> {
            let header = ptr.cast::<RawDriverHeader<C>>();

            header
                .as_ref()
                .raw_driver
                .tcp_listen(laddr)
                .map(|p| p.into())
        }

        unsafe fn tcp_connect<C: RawDriver>(
            ptr: NonNull<RawDriverVTable>,
            raddr: SocketAddr,
        ) -> io::Result<TcpStream> {
            let header = ptr.cast::<RawDriverHeader<C>>();

            header
                .as_ref()
                .raw_driver
                .tcp_connect(raddr)
                .map(|p| p.into())
        }

        RawDriverVTable {
            new_poller: new_poller::<C>,
            udp_bind: udp_bind::<C>,
            tcp_listen: tcp_listen::<C>,
            tcp_connect: tcp_connect::<C>,
        }
    }
}

/// Hala io runtime driver object which handle varaint io objects creating/drop
#[derive(Clone)]
pub struct Driver {
    ptr: NonNull<RawDriverVTable>,
}

impl Driver {
    /// Create new driver from [`RawDriver`]
    pub fn new<D: RawDriver>(raw_driver: D) -> Self {
        let boxed = Box::new(RawDriverHeader::<D> {
            vtable: RawDriverVTable::new::<D>(),
            raw_driver,
        });

        let ptr = unsafe { NonNull::new_unchecked(Box::into_raw(boxed) as *mut RawDriverVTable) };

        Self { ptr }
    }

    /// Create new io events [`Poller`]
    pub fn new_poller(&self) -> io::Result<Poller> {
        unsafe { (self.ptr.as_ref().new_poller)(self.ptr) }
    }

    /// Create new `UdpSocket` and bind it to `laddr`, see [`more`](RawDriver::udp_bind)
    pub fn udp_bind(&self, laddr: SocketAddr) -> io::Result<UdpSocket> {
        unsafe { (self.ptr.as_ref().udp_bind)(self.ptr, laddr) }
    }

    /// Create new `TcpListener` and listen on `laddr`, see [`more`](RawDriver::tcp_listen)
    pub fn tcp_listen(&self, laddr: SocketAddr) -> io::Result<TcpListener> {
        unsafe { (self.ptr.as_ref().tcp_listen)(self.ptr, laddr) }
    }

    /// Create new `TcpStream` and connect to `raddr`, see [`more`](RawDriver::tcp_listen)
    pub fn tcp_connect(&self, raddr: SocketAddr) -> io::Result<TcpStream> {
        unsafe { (self.ptr.as_ref().tcp_connect)(self.ptr, raddr) }
    }
}

unsafe impl Sync for Driver {}
unsafe impl Send for Driver {}
