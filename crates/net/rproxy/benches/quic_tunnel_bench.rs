use async_trait::async_trait;
use hala_rproxy::{
    handshake::{HandshakeContext, Handshaker, TunnelOpenConfiguration},
    quic::QuicTransport,
    transport::{PathInfo, TransportConfig},
    tunnel::TunnelFactoryManager,
};

use std::{io, net::SocketAddr, time::Instant};

use bytes::BytesMut;
use futures::{channel::mpsc, AsyncWriteExt, SinkExt, StreamExt};
use hala_future::executor::{block_on, future_spawn};
use hala_quic::{Config, QuicConn, QuicListener, QuicStream};

fn mock_config(is_server: bool, max_datagram_size: usize) -> Config {
    use std::path::Path;

    let mut config = Config::new().unwrap();

    config.verify_peer(true);

    // if is_server {
    let root_path = Path::new(env!("CARGO_MANIFEST_DIR"));

    log::debug!("test run dir {:?}", root_path);

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

    config.set_max_idle_timeout(1000);
    config.set_max_recv_udp_payload_size(max_datagram_size);
    config.set_max_send_udp_payload_size(max_datagram_size);
    config.set_initial_max_data(10_000_000);
    config.set_initial_max_stream_data_bidi_local(10_000_000);
    config.set_initial_max_stream_data_bidi_remote(10_000_000);
    config.set_initial_max_streams_bidi(9);
    config.set_initial_max_streams_uni(9);
    config.set_disable_active_migration(false);
    // config.set_cc_algorithm(CongestionControlAlgorithm::Reno);

    config
}

async fn create_echo_server(max_streams: u64) -> SocketAddr {
    let mut config = mock_config(true, 1370);

    config.set_initial_max_streams_bidi(max_streams);

    let listener: QuicListener = QuicListener::bind("127.0.0.1:0", config).unwrap();

    let listener_cloned = listener.clone();

    future_spawn(async move {
        while let Some(conn) = listener_cloned.accept().await {
            future_spawn(handle_echo_conn(conn));
        }
    });

    let addr = listener.local_addrs().next().unwrap();

    addr.clone()
}

async fn handle_echo_conn(conn: QuicConn) {
    while let Some(stream) = conn.accept_stream().await {
        future_spawn(handle_echo_stream(stream));
    }
}

async fn handle_echo_stream(mut stream: QuicStream) {
    let mut buf = vec![0; 1370];

    loop {
        log::trace!("{:?} begin read", stream);
        let (read_size, fin) = stream.stream_recv(&mut buf).await.unwrap();
        log::trace!("{:?} end read", stream);

        log::trace!("{:?} begin write", stream);
        stream.write_all(&buf[..read_size]).await.unwrap();
        log::trace!("{:?} end write", stream);

        if fin {
            return;
        }
    }
}

fn mock_create_config(_flag: &TransportConfig, max_packet_len: usize) -> Config {
    mock_config(false, max_packet_len)
}

struct MockHandshaker {
    raddr: SocketAddr,
}

#[async_trait]
impl Handshaker for MockHandshaker {
    async fn handshake(
        &self,
        cx: HandshakeContext,
    ) -> io::Result<(HandshakeContext, TunnelOpenConfiguration)> {
        let raddr = self.raddr.clone();
        let max_packet_len = cx.max_packet_len;
        let max_cache_len = cx.max_cache_len;

        Ok((
            cx,
            TunnelOpenConfiguration {
                max_packet_len,
                max_cache_len,
                tunnel_service_id: "QuicTransport".into(),
                transport_config: TransportConfig::Quic(
                    vec!["127.0.0.1:0".parse().unwrap()],
                    vec![raddr],
                ),
            },
        ))
    }
}

fn mock_tm(raddr: SocketAddr, max_packet_len: usize) -> TunnelFactoryManager {
    let transport_manager = TunnelFactoryManager::new(MockHandshaker { raddr });

    transport_manager.register(QuicTransport::new("QuicTransport", move |flag| {
        mock_create_config(flag, max_packet_len)
    }));

    transport_manager
}

async fn mock_client(
    tm: &TunnelFactoryManager,
    cache_queue_len: usize,
) -> (mpsc::Sender<BytesMut>, mpsc::Receiver<BytesMut>) {
    let (sender, forward_receiver) = mpsc::channel(cache_queue_len);
    let (backward_sender, receiver) = mpsc::channel(cache_queue_len);

    let cx = HandshakeContext {
        path: PathInfo::None,
        max_cache_len: 1024,
        max_packet_len: 1370,
        forward: forward_receiver,
        backward: backward_sender,
    };

    tm.handshake(cx).await.unwrap();

    (sender, receiver)
}

fn main() {
    let raddr = block_on(async { create_echo_server(200).await }, 10);

    println!("quic_bench");

    block_on(echo_single_client(raddr, 10000), 10);
    block_on(echo_mult_client(raddr, 100), 10);
    block_on(echo_mult_client_multi_thread(raddr, 100), 10);
}

async fn echo_single_client(raddr: SocketAddr, times: u32) {
    let cache_queue_len = 1024;
    let max_packet_len = 1350;

    let tm = mock_tm(raddr, max_packet_len);

    let (mut sender, mut receiver) = mock_client(&tm, cache_queue_len).await;

    let start = Instant::now();

    for i in 0..times {
        let send_data = format!("Hello world, {}", i);

        sender
            .send(BytesMut::from(send_data.as_bytes()))
            .await
            .unwrap();

        let buf = receiver.next().await.unwrap();

        assert_eq!(&buf, send_data.as_bytes());
    }

    println!("\techo_single_client: {:?}", start.elapsed() / times);
}

async fn echo_mult_client(raddr: SocketAddr, times: u32) {
    let cache_queue_len = 1024;
    let max_packet_len = 1350;

    let tm = mock_tm(raddr, max_packet_len);

    let start = Instant::now();

    for i in 0..times {
        let (mut sender, mut receiver) = mock_client(&tm, cache_queue_len).await;

        for j in 0..100 {
            let send_data = format!("Hello world, {} {}", i, j);

            sender
                .send(BytesMut::from(send_data.as_bytes()))
                .await
                .unwrap();

            let buf = receiver.next().await.unwrap();

            assert_eq!(&buf, send_data.as_bytes());
        }
    }

    println!("\techo_mult_client: {:?}", start.elapsed() / times);
}

async fn echo_mult_client_multi_thread(raddr: SocketAddr, times: u32) {
    // pretty_env_logger::init();

    let cache_queue_len = 1024;
    let max_packet_len = 1350;

    let tm = mock_tm(raddr, max_packet_len);

    let (sx, mut rx) = mpsc::channel(0);

    let clients = 100;

    let start = Instant::now();

    for i in 0..clients {
        let (mut sender, mut receiver) = mock_client(&tm, cache_queue_len).await;

        let mut sx = sx.clone();

        future_spawn(async move {
            log::trace!("start {}", i);

            for j in 0..times {
                let send_data = format!("Hello world, {} {}", i, j);

                sender
                    .send(BytesMut::from(send_data.as_bytes()))
                    .await
                    .unwrap();

                let buf = receiver.next().await.unwrap();

                assert_eq!(&buf, send_data.as_bytes());
            }

            sx.send(()).await.unwrap();

            log::trace!("finished {}", i);
        })
    }

    for _ in 0..clients {
        rx.next().await.unwrap();
    }

    println!(
        "\techo_mult_client_multi_thread: {:?}",
        start.elapsed() / times
    );
}
