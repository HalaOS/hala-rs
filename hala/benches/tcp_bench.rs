use std::{net::SocketAddr, time::Instant};

use futures::{AsyncReadExt, AsyncWriteExt};
use hala_future::executor::{block_on, future_spawn};
use hala_tcp::{TcpListener, TcpStream};

fn create_echo_server() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();

    let raddr = listener.local_addr().unwrap();

    future_spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    future_spawn(handle_echo_stream(stream));
                }
                Err(_) => break,
            }
        }
    });

    raddr
}

async fn handle_echo_stream(mut stream: TcpStream) {
    let mut buf = vec![0; 1370];

    loop {
        let read_size = stream.read(&mut buf).await.unwrap();

        if read_size == 0 {
            break;
        }

        stream.write_all(&buf[..read_size]).await.unwrap();
    }
}

fn main() {
    let raddr = block_on(async { create_echo_server() }, 10);

    println!("tcp_bench");

    block_on(test_echo(raddr, 10000), 10);
}

async fn test_echo(raddr: SocketAddr, times: u32) {
    let mut stream = TcpStream::connect(raddr).unwrap();

    let start = Instant::now();

    for i in 0..times {
        let send_data = format!("Hello world, {}", i);

        stream.write(send_data.as_bytes()).await.unwrap();

        let mut buf = [0; 1024];

        let read_size = stream.read(&mut buf).await.unwrap();

        assert_eq!(&buf[..read_size], send_data.as_bytes());
    }

    println!("\ttest_echo: {:?}", start.elapsed() / times);
}
