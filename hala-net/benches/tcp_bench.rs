use std::sync::Once;

use divan::Bencher;

use futures::executor::LocalPool;
use futures::{task::*, AsyncReadExt, AsyncWriteExt};
use hala_io_driver::*;
use hala_io_util::*;
use hala_net::*;

use std::io::{Read, Write};

fn main() {
    divan::main();
}

static INIT: Once = Once::new();

// Define a `fibonacci` function and register it for benchmarking.
#[divan::bench(sample_size = 1000)]
fn async_tcp_echo(bench: Bencher) {
    INIT.call_once(|| {
        _ = register_driver(mio_driver());
        _ = register_local_poller();
    });

    let _guard = PollLoopGuard::new(None).unwrap();

    let mut local_pool = LocalPool::new();

    let spawner = local_pool.spawner();

    let echo_data = b"hello";

    let tcp_listener = TcpListener::bind("127.0.0.1:0").unwrap();

    let laddr = tcp_listener.local_addr().unwrap();

    let mut client = TcpStream::connect(&[laddr].as_slice()).unwrap();

    spawner
        .spawn(async move {
            let (mut conn, _) = tcp_listener.accept().await.unwrap();

            loop {
                let mut buf = [0; 1024];

                let read_size = conn.read(&mut buf).await.unwrap();

                assert_eq!(read_size, echo_data.len());

                let write_size = conn.write(&buf[..read_size]).await.unwrap();

                assert_eq!(write_size, echo_data.len());
            }
        })
        .unwrap();

    bench.bench_local(|| {
        local_pool.run_until(async {
            let write_size = client.write(echo_data).await.unwrap();

            assert_eq!(write_size, echo_data.len());

            let mut buf = [0; 1024];

            let read_size = client.read(&mut buf).await.unwrap();

            assert_eq!(read_size, echo_data.len());
        })
    });
}

#[divan::bench(sample_size = 1000)]
fn tcp_echo(bench: Bencher) {
    let echo_data = b"hello";

    let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();

    let laddr = server.local_addr().unwrap();

    std::thread::spawn(move || {
        let (mut conn, _) = server.accept().unwrap();
        loop {
            let mut buf = [0; 1024];

            let read_size = conn.read(&mut buf).unwrap();

            assert_eq!(read_size, echo_data.len());

            let write_size = conn.write(&buf[..read_size]).unwrap();

            assert_eq!(write_size, echo_data.len());
        }
    });

    let mut client = std::net::TcpStream::connect(laddr).unwrap();

    bench.bench_local(|| {
        let write_size = client.write(echo_data).unwrap();

        assert_eq!(write_size, echo_data.len());

        let mut buf = [0; 1024];

        let read_size = client.read(&mut buf).unwrap();

        assert_eq!(read_size, echo_data.len());
    });
}
