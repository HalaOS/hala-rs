use std::{
    io::{Read, Write},
    sync::Arc,
};

use divan::Bencher;
use futures::{AsyncReadExt, AsyncWriteExt};
use hala_io_util::{get_local_poller, io_spawn, local_io_spawn};
use hala_net::{TcpListener, TcpStream};

fn main() {
    divan::main();
}

#[divan::bench]
fn local_tcp_echo(bench: Bencher) {
    let echo_data = b"hello";

    let tcp_listener = hala_io_util::local_block_on(async {
        Arc::new(TcpListener::bind_with("127.0.0.1:0", get_local_poller().unwrap()).unwrap())
    });

    let laddr = tcp_listener.local_addr().unwrap();

    bench.bench_local(|| {
        let tcp_listener = tcp_listener.clone();
        let laddr = laddr.clone();

        hala_io_util::local_block_on(async move {
            local_io_spawn(async move {
                let mut tcp_stream =
                    TcpStream::connect_with(&[laddr].as_slice(), get_local_poller()?).unwrap();

                let mut buf = [0; 1024];

                let read_size = tcp_stream.read(&mut buf).await.unwrap();

                assert_eq!(read_size, echo_data.len());

                let write_size = tcp_stream.write(&buf[..read_size]).await.unwrap();

                assert_eq!(write_size, echo_data.len());

                Ok(())
            })
            .unwrap();

            let (mut conn, _) = tcp_listener
                .accept_with(get_local_poller().unwrap())
                .await
                .unwrap();

            log::trace!("tcp_listener accept");

            let write_size = conn.write(echo_data).await.unwrap();

            assert_eq!(write_size, echo_data.len());

            let mut buf = [0; 1024];

            let read_size = conn.read(&mut buf).await.unwrap();

            assert_eq!(read_size, echo_data.len());
        });
    });
}

#[divan::bench]
fn tcp_echo(bench: Bencher) {
    let echo_data = b"hello";

    let tcp_listener = hala_io_util::block_on(
        async { Arc::new(TcpListener::bind("127.0.0.1:0").unwrap()) },
        1,
    );

    let laddr = tcp_listener.local_addr().unwrap();

    bench.bench_local(|| {
        let tcp_listener = tcp_listener.clone();
        let laddr = laddr.clone();

        hala_io_util::block_on(
            async move {
                io_spawn(async move {
                    let mut tcp_stream = TcpStream::connect(&[laddr].as_slice()).unwrap();

                    let mut buf = [0; 1024];

                    let read_size = tcp_stream.read(&mut buf).await.unwrap();

                    assert_eq!(read_size, echo_data.len());

                    let write_size = tcp_stream.write(&buf[..read_size]).await.unwrap();

                    assert_eq!(write_size, echo_data.len());

                    Ok(())
                })
                .unwrap();

                let (mut conn, _) = tcp_listener.accept().await.unwrap();

                log::trace!("tcp_listener accept");

                let write_size = conn.write(echo_data).await.unwrap();

                assert_eq!(write_size, echo_data.len());

                let mut buf = [0; 1024];

                let read_size = conn.read(&mut buf).await.unwrap();

                assert_eq!(read_size, echo_data.len());
            },
            10,
        );
    });
}

#[divan::bench]
fn std_tcp_echo(bench: Bencher) {
    let echo_data = b"hello";

    let tcp_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();

    let laddr = tcp_listener.local_addr().unwrap();

    std::thread::spawn(move || loop {
        let (mut conn, _) = tcp_listener.accept().unwrap();

        log::trace!("tcp_listener accept");

        let write_size = conn.write(echo_data).unwrap();

        assert_eq!(write_size, echo_data.len());

        let mut buf = [0; 1024];

        let read_size = conn.read(&mut buf).unwrap();

        assert_eq!(read_size, echo_data.len());
    });

    bench.bench_local(|| {
        let laddr = laddr.clone();

        let mut tcp_stream = std::net::TcpStream::connect(&[laddr].as_slice()).unwrap();

        let mut buf = [0; 1024];

        let read_size = tcp_stream.read(&mut buf).unwrap();

        assert_eq!(read_size, echo_data.len());

        let write_size = tcp_stream.write(&buf[..read_size]).unwrap();

        assert_eq!(write_size, echo_data.len());
    });
}
