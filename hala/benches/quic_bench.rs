use std::{net::SocketAddr, time::Instant};

use futures::{AsyncReadExt, AsyncWriteExt};
use hala_future::executor::{block_on, future_spawn};
use hala_quic::{Config, QuicConn, QuicListener, QuicStream};

fn mock_config(is_server: bool, max_datagram_size: usize) -> Config {
    use std::path::Path;

    let mut config = Config::new().unwrap();

    config.verify_peer(true);

    // if is_server {
    let root_path = Path::new(env!("CARGO_MANIFEST_DIR"));

    if is_server {
        config
            .load_cert_chain_from_pem_file(root_path.join("cert/server.crt").to_str().unwrap())
            .unwrap();

        config
            .load_priv_key_from_pem_file(root_path.join("cert/server.key").to_str().unwrap())
            .unwrap();
    } else {
        config
            .load_cert_chain_from_pem_file(root_path.join("cert/client.crt").to_str().unwrap())
            .unwrap();

        config
            .load_priv_key_from_pem_file(root_path.join("cert/client.key").to_str().unwrap())
            .unwrap();
    }

    config
        .load_verify_locations_from_file(root_path.join("cert/hala_ca.pem").to_str().unwrap())
        .unwrap();

    config
        .set_application_protos(&[b"hq-interop", b"hq-29", b"hq-28", b"hq-27", b"http/0.9"])
        .unwrap();

    config.set_max_idle_timeout(5000);
    config.set_max_recv_udp_payload_size(max_datagram_size);
    config.set_max_send_udp_payload_size(max_datagram_size);
    config.set_initial_max_data(10_000_000);
    config.set_initial_max_stream_data_bidi_local((max_datagram_size * 10) as u64);
    config.set_initial_max_stream_data_bidi_remote((max_datagram_size * 10) as u64);
    config.set_initial_max_streams_bidi(9);
    config.set_initial_max_streams_uni(9);
    config.set_disable_active_migration(false);

    config
}

fn create_echo_server() -> SocketAddr {
    let listener = QuicListener::bind("127.0.0.1:0", mock_config(true, 1370)).unwrap();

    let raddr = listener.local_addrs().next().unwrap().clone();

    future_spawn(async move {
        while let Some(conn) = listener.accept().await {
            future_spawn(handle_conn(conn));
        }
    });

    raddr
}

async fn handle_conn(conn: QuicConn) {
    while let Some(stream) = conn.accept_stream().await {
        future_spawn(handle_stream(stream));
    }
}

async fn handle_stream(mut stream: QuicStream) {
    let mut buf = vec![0; 1370];

    loop {
        let (read_size, fin) = stream.stream_recv(&mut buf).await.unwrap();

        if fin {
            break;
        }

        stream.write_all(&buf[..read_size]).await.unwrap();
    }
}

fn main() {
    pretty_env_logger::init_timed();

    let raddr = block_on(async { create_echo_server() }, 10);

    println!("tcp_bench");

    block_on(test_echo(raddr, 10000), 10);
}

async fn test_echo(raddr: SocketAddr, times: u32) {
    let conn = QuicConn::connect("127.0.0.1:0", raddr, &mut mock_config(false, 1370))
        .await
        .unwrap();

    let mut stream = conn.open_stream().await.unwrap();

    let start = Instant::now();

    for i in 0..times {
        let send_data = format!("Hello world, {}", i);

        stream.write(send_data.as_bytes()).await.unwrap();

        let mut buf = [0; 1024];

        let read_size = stream.read(&mut buf).await.unwrap();

        assert_eq!(&buf[..read_size], send_data.as_bytes());
    }

    println!("\ttest_echo: {:?}", start.elapsed() / times,);
}
