use std::sync::Once;

use divan::Bencher;

use futures::executor::LocalPool;
use futures::task::*;
use hala_io_driver::*;
use hala_net::*;
use rand::seq::SliceRandom;

fn main() {
    divan::main();
}

static INIT: Once = Once::new();

#[divan::bench(sample_size = 1000)]
fn async_udp_echo(bench: Bencher) {
    INIT.call_once(|| {
        _ = register_driver(mio_driver());
    });

    let _guard = PollGuard::new(None).unwrap();

    let mut local_pool = LocalPool::new();

    let spawner = local_pool.spawner();

    let echo_data = b"hello";

    let server = UdpSocket::bind("127.0.0.1:0").unwrap();

    let laddr = server.local_addr().unwrap();

    let client = UdpSocket::bind("127.0.0.1:0").unwrap();

    spawner
        .spawn(async move {
            loop {
                let mut buf = [0; 1024];

                let (read_size, raddr) = server.recv_from(&mut buf).await.unwrap();

                assert_eq!(read_size, echo_data.len());

                let write_size = server.send_to(&buf[..read_size], raddr).await.unwrap();

                assert_eq!(write_size, echo_data.len());
            }
        })
        .unwrap();

    bench.bench_local(|| {
        local_pool.run_until(async {
            let write_size = client.send_to(echo_data, laddr).await.unwrap();

            assert_eq!(write_size, echo_data.len());

            let mut buf = [0; 1024];

            let (read_size, raddr) = client.recv_from(&mut buf).await.unwrap();

            assert_eq!(read_size, echo_data.len());

            assert_eq!(raddr, laddr);
        })
    });
}

#[divan::bench(sample_size = 1000)]
fn async_udpgroup_echo(bench: Bencher) {
    INIT.call_once(|| {
        _ = register_driver(mio_driver());
    });

    let _guard = PollGuard::new(None).unwrap();

    let mut local_pool = LocalPool::new();

    let spawner = local_pool.spawner();

    let echo_data = b"hello";

    let ports = 10000..10100;

    let udp_server = UdpGroup::bind("127.0.0.1".parse().unwrap(), ports.clone()).unwrap();

    let udp_client = UdpSocket::bind("127.0.0.1:0").unwrap();

    let ports = ports.into_iter().collect::<Vec<_>>();

    spawner
        .spawn_local(async move {
            loop {
                let mut buf = [0; 1024];

                let (_, read_size, raddr) = udp_server.recv_from(&mut buf).await.unwrap();

                assert_eq!(read_size, echo_data.len());

                let (_, write_size) = udp_server.send_to(&buf[..read_size], raddr).await.unwrap();

                assert_eq!(write_size, echo_data.len());
            }
        })
        .unwrap();

    bench.bench_local(|| {
        local_pool.run_until(async {
            let port = ports.choose(&mut rand::thread_rng()).clone().unwrap();

            let write_size = udp_client
                .send_to(echo_data, format!("127.0.0.1:{}", port))
                .await
                .unwrap();

            assert_eq!(write_size, echo_data.len());

            let mut buf = [0; 1024];

            let (read_size, _) = udp_client.recv_from(&mut buf).await.unwrap();

            assert_eq!(read_size, echo_data.len());
        })
    });
}

#[divan::bench(sample_size = 1000)]
fn udp_echo(bench: Bencher) {
    let echo_data = b"hello";

    let server = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();

    let laddr = server.local_addr().unwrap();

    let client = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();

    std::thread::spawn(move || loop {
        let mut buf = [0; 1024];

        let (read_size, raddr) = server.recv_from(&mut buf).unwrap();

        assert_eq!(read_size, echo_data.len());

        let write_size = server.send_to(&buf[..read_size], raddr).unwrap();

        assert_eq!(write_size, echo_data.len());
    });

    bench.bench_local(|| {
        let write_size = client.send_to(echo_data, laddr).unwrap();

        assert_eq!(write_size, echo_data.len());

        let mut buf = [0; 1024];

        let (read_size, raddr) = client.recv_from(&mut buf).unwrap();

        assert_eq!(read_size, echo_data.len());

        assert_eq!(raddr, laddr);
    });
}
