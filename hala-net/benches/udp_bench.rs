use std::{rc::Rc, sync::Arc};

use divan::Bencher;
use hala_io_util::{get_local_poller, io_spawn, local_io_spawn};
use hala_net::UdpSocket;

fn main() {
    divan::main();
}

#[divan::bench]
fn local_udp_echo(bench: Bencher) {
    let echo_data = b"hello";

    let server = hala_io_util::local_block_on(async {
        Rc::new(UdpSocket::bind_with("127.0.0.1:0", get_local_poller().unwrap()).unwrap())
    });

    let client = hala_io_util::local_block_on(async {
        Rc::new(UdpSocket::bind_with("127.0.0.1:0", get_local_poller().unwrap()).unwrap())
    });

    let laddr = server.local_addr().unwrap();

    bench.bench_local(|| {
        let server = server.clone();
        let client = client.clone();
        let laddr = laddr.clone();

        hala_io_util::local_block_on(async move {
            local_io_spawn(async move {
                let mut buf = [0; 1024];

                let (read_size, raddr) = server.recv_from(&mut buf).await.unwrap();

                assert_eq!(read_size, echo_data.len());

                let write_size = server.send_to(&buf[..read_size], raddr).await.unwrap();

                assert_eq!(write_size, echo_data.len());

                Ok(())
            })
            .unwrap();

            let write_size = client.send_to(echo_data, laddr).await.unwrap();

            assert_eq!(write_size, echo_data.len());

            let mut buf = [0; 1024];

            let (read_size, raddr) = client.recv_from(&mut buf).await.unwrap();

            assert_eq!(read_size, echo_data.len());

            assert_eq!(raddr, laddr);
        });
    });
}

#[divan::bench]
fn udp_echo(bench: Bencher) {
    let echo_data = b"hello";

    let server =
        hala_io_util::local_block_on(async { Arc::new(UdpSocket::bind("127.0.0.1:0").unwrap()) });

    let client =
        hala_io_util::local_block_on(async { Arc::new(UdpSocket::bind("127.0.0.1:0").unwrap()) });

    let laddr = server.local_addr().unwrap();

    bench.bench_local(|| {
        let server = server.clone();
        let client = client.clone();
        let laddr = laddr.clone();

        hala_io_util::block_on(
            async move {
                io_spawn(async move {
                    let mut buf = [0; 1024];

                    let (read_size, raddr) = server.recv_from(&mut buf).await.unwrap();

                    assert_eq!(read_size, echo_data.len());

                    let write_size = server.send_to(&buf[..read_size], raddr).await.unwrap();

                    assert_eq!(write_size, echo_data.len());

                    Ok(())
                })
                .unwrap();

                let write_size = client.send_to(echo_data, laddr).await.unwrap();

                assert_eq!(write_size, echo_data.len());

                let mut buf = [0; 1024];

                let (read_size, raddr) = client.recv_from(&mut buf).await.unwrap();

                assert_eq!(read_size, echo_data.len());

                assert_eq!(raddr, laddr);
            },
            10,
        );
    });
}

#[divan::bench]
fn std_udp_echo(bench: Bencher) {
    let echo_data = b"hello";

    let server = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let client = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();

    let laddr = server.local_addr().unwrap();

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
