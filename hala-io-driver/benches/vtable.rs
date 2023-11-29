use std::io;

use divan::Bencher;
use hala_io_driver::{Driver, RawDriver, RawPoller, RawTcpListener, RawTcpStream, RawUdpSocket};

fn main() {
    // Run registered benchmarks.
    divan::main();
}

struct UdpSocketImpl {}

impl RawUdpSocket for UdpSocketImpl {}

struct TcpListenerImpl {}

impl RawTcpListener for TcpListenerImpl {}

struct TcpStreamImpl {}

impl RawTcpStream for TcpStreamImpl {}

struct PollerImpl {}

impl RawPoller for PollerImpl {}
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
