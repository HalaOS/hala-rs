use std::{net::SocketAddr, path::Path, sync::Arc};

use futures::{AsyncReadExt, AsyncWriteExt};
use hala_future::executor::future_spawn;
use hala_io::test::io_test;
use hala_tcp::{TcpListener, TcpStream};
use hala_tls::{accept, connect, SslAcceptor, SslConnector, SslFiletype, SslMethod, SslStream};

fn create_echo_server() -> SocketAddr {
    let root_path = Path::new(env!("CARGO_MANIFEST_DIR"));

    let mut acceptor = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();

    acceptor
        .set_private_key_file(root_path.join("cert/server.key"), SslFiletype::PEM)
        .unwrap();
    acceptor
        .set_certificate_chain_file(root_path.join("cert/server.crt"))
        .unwrap();

    acceptor.check_private_key().unwrap();
    let acceptor = Arc::new(acceptor.build());

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();

    let raddr = listener.local_addr().unwrap();

    future_spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let stream = accept(&acceptor, stream).await.unwrap();
                    future_spawn(handle_echo_stream(stream));
                }
                Err(_) => break,
            }
        }
    });

    raddr
}

async fn handle_echo_stream(mut stream: SslStream<TcpStream>) {
    let mut buf = vec![0; 1370];

    loop {
        let read_size = stream.read(&mut buf).await.unwrap();

        if read_size == 0 {
            break;
        }

        stream.write_all(&buf[..read_size]).await.unwrap();
    }
}

#[hala_test::test(io_test)]
async fn test_echo() {
    let root_path = Path::new(env!("CARGO_MANIFEST_DIR"));

    let raddr = create_echo_server();

    let stream = TcpStream::connect(raddr).unwrap();

    let mut config = SslConnector::builder(SslMethod::tls()).unwrap();

    config
        .set_ca_file(root_path.join("cert/hala_ca.pem"))
        .unwrap();

    let config = config.build().configure().unwrap();

    let mut stream = connect(config, "hala.quic", stream).await.unwrap();

    for i in 0..10000 {
        let send_data = format!("Hello world, {}", i);

        stream.write(send_data.as_bytes()).await.unwrap();

        let mut buf = [0; 1024];

        let read_size = stream.read(&mut buf).await.unwrap();

        assert_eq!(&buf[..read_size], send_data.as_bytes());
    }
}
