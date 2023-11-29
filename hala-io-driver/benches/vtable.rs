use std::io;

use divan::Bencher;
use hala_io_driver::{Driver, RawDriver, RawPoller, RawTcpListener, RawTcpStream, RawUdpSocket};

fn main() {
    // Run registered benchmarks.
    divan::main();
}

struct UdpSocketImpl {}

#[allow(unused)]
impl RawUdpSocket for UdpSocketImpl {
    fn send_to(&self, buf: &[u8], raddr: std::net::SocketAddr) -> io::Result<usize> {
        todo!()
    }

    fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, std::net::SocketAddr)> {
        todo!()
    }

    fn try_clone(&self) -> io::Result<Self>
    where
        Self: Sized,
    {
        todo!()
    }
}

struct TcpListenerImpl {}

impl RawTcpListener for TcpListenerImpl {
    type TcpStream = TcpStreamImpl;

    fn accept(&self) -> io::Result<(Self::TcpStream, std::net::SocketAddr)> {
        todo!()
    }
}

struct TcpStreamImpl {}

#[allow(unused)]
impl RawTcpStream for TcpStreamImpl {
    fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        todo!()
    }

    fn write(&self, buf: &[u8]) -> io::Result<usize> {
        todo!()
    }
}

struct PollerImpl {}

impl RawPoller for PollerImpl {
    fn poll_once(
        &self,
        _timeout: Option<std::time::Duration>,
    ) -> io::Result<Vec<hala_io_driver::Event>> {
        Ok(vec![])
    }
}

#[derive(Clone)]
struct DriverImpl {}

impl RawDriver for DriverImpl {
    type Poller = PollerImpl;

    type UdpSocket = UdpSocketImpl;

    type TcpListener = TcpListenerImpl;

    type TcpStream = TcpStreamImpl;

    #[inline(never)]
    fn new_poller(&self) -> io::Result<Self::Poller> {
        Ok(PollerImpl {})
    }

    #[inline(never)]
    fn udp_bind(&self, _laddr: std::net::SocketAddr) -> io::Result<Self::UdpSocket> {
        Ok(UdpSocketImpl {})
    }

    #[inline(never)]
    fn tcp_listen(&self, _laddr: std::net::SocketAddr) -> io::Result<Self::TcpListener> {
        Ok(TcpListenerImpl {})
    }

    #[inline(never)]
    fn tcp_connect(&self, _raddr: std::net::SocketAddr) -> io::Result<Self::TcpStream> {
        Ok(TcpStreamImpl {})
    }

    fn try_clone(&self) -> io::Result<Self>
    where
        Self: Sized,
    {
        Ok(self.clone())
    }
}

#[divan::bench(sample_count = 10000)]
fn vtable_new_poller(bencher: Bencher) {
    let driver = Driver::new(DriverImpl {});
    bencher.bench(|| driver.new_poller().unwrap())
}

#[divan::bench(sample_count = 10000)]
fn new_poller(bencher: Bencher) {
    let driver = DriverImpl {};
    bencher.bench(|| driver.new_poller().unwrap())
}

#[divan::bench(sample_count = 10000)]
fn vtable_udp_bind(bencher: Bencher) {
    let addr = "127.0.0.1:0".parse().unwrap();

    let driver = Driver::new(DriverImpl {});
    bencher.bench(|| driver.udp_bind(addr).unwrap())
}

#[divan::bench(sample_count = 10000)]
fn new_udp_bind(bencher: Bencher) {
    let addr = "127.0.0.1:0".parse().unwrap();

    let driver = DriverImpl {};
    bencher.bench(|| driver.udp_bind(addr).unwrap())
}

#[divan::bench(sample_count = 10000)]
fn vtable_tcp_listen(bencher: Bencher) {
    let addr = "127.0.0.1:0".parse().unwrap();

    let driver = Driver::new(DriverImpl {});
    bencher.bench(|| driver.tcp_listen(addr).unwrap())
}

#[divan::bench(sample_count = 10000)]
fn new_tcp_listen(bencher: Bencher) {
    let addr = "127.0.0.1:0".parse().unwrap();

    let driver = DriverImpl {};
    bencher.bench(|| driver.tcp_listen(addr).unwrap())
}

#[divan::bench(sample_count = 10000)]
fn vtable_tcp_connect(bencher: Bencher) {
    let addr = "127.0.0.1:0".parse().unwrap();

    let driver = Driver::new(DriverImpl {});
    bencher.bench(|| driver.tcp_connect(addr).unwrap())
}

#[divan::bench(sample_count = 10000)]
fn new_tcp_connect(bencher: Bencher) {
    let addr = "127.0.0.1:0".parse().unwrap();

    let driver = DriverImpl {};
    bencher.bench(|| driver.tcp_connect(addr).unwrap())
}
