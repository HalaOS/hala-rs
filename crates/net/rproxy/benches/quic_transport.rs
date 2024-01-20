use std::{io, net::SocketAddr, sync::OnceLock, time::Instant};

use bytes::BytesMut;
use futures::{channel::mpsc, AsyncWriteExt, SinkExt, StreamExt};
use hala_future::executor::{block_on, future_spawn};
use hala_quic::{Config, QuicConn, QuicListener, QuicStream};
use hala_rproxy::{
    handshake::{HandshakeContext, HandshakeResult, Handshaker, Protocol},
    quic::QuicTransport,
    transport::{ChannelOpenFlag, TransportManager},
};

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

    config
}

async fn create_echo_server(max_streams: u64) -> QuicListener {
    let mut config = mock_config(true, 1370);

    config.set_initial_max_streams_bidi(max_streams);

    let listener: QuicListener = QuicListener::bind("127.0.0.1:0", config).unwrap();

    let listener_cloned = listener.clone();

    future_spawn(async move {
        while let Some(conn) = listener_cloned.accept().await {
            future_spawn(handle_echo_conn(conn));
        }
    });

    listener
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

fn mock_create_config(_flag: &ChannelOpenFlag, max_packet_len: usize) -> Config {
    mock_config(false, max_packet_len)
}

struct MockHandshaker {
    raddr: SocketAddr,
}

impl Handshaker for MockHandshaker {
    fn handshake(
        &self,
        forward_cx: HandshakeContext,
    ) -> futures::prelude::future::BoxFuture<'static, io::Result<HandshakeResult>> {
        let raddr = self.raddr.clone();

        Box::pin(async move {
            Ok(HandshakeResult {
                context: forward_cx,
                transport_id: "QuicTransport".into(),
                channel_open_flag: ChannelOpenFlag::RemoteAddresses(vec![raddr]),
            })
        })
    }
}

fn mock_tm(raddr: SocketAddr, cache_queue_len: usize, max_packet_len: usize) -> TransportManager {
    let transport_manager =
        TransportManager::new(MockHandshaker { raddr }, cache_queue_len, max_packet_len);

    transport_manager.register(QuicTransport::new("QuicTransport", move |flag| {
        mock_create_config(flag, max_packet_len)
    }));

    transport_manager
}

const SERVER_ADDR: OnceLock<SocketAddr> = OnceLock::new();

async fn mock_client(
    tm: &TransportManager,
    cache_queue_len: usize,
) -> (mpsc::Sender<BytesMut>, mpsc::Receiver<BytesMut>) {
    let (sender, forward_receiver) = mpsc::channel(cache_queue_len);
    let (backward_sender, receiver) = mpsc::channel(cache_queue_len);

    let cx = HandshakeContext {
        from: "".into(),
        to: "".into(),
        protocol: Protocol::Other("test".into()),
        forward: forward_receiver,
        backward: backward_sender,
    };

    tm.handshake(cx).await.unwrap();

    (sender, receiver)
}

async fn setup() -> SocketAddr {
    let listener = create_echo_server(10).await;

    let addr = listener.local_addrs().next().unwrap().clone();

    addr
}

fn main() {
    // pretty_env_logger::init_timed();
    let raddr = SERVER_ADDR
        .get_or_init(|| {
            let addr = block_on(setup(), 10);

            addr
        })
        .clone();

    divan::main();

    println!("quic_transport");

    block_on(echo_single_client(raddr.clone(), 10000), 10);
}

const CACHE_QUEUE_LEN: usize = 1024;
const MAX_PACKET_LEN: usize = 1370;

async fn echo_single_client(raddr: SocketAddr, times: usize) {
    let tm = mock_tm(raddr, CACHE_QUEUE_LEN, MAX_PACKET_LEN);

    let (mut sender, mut receiver) = mock_client(&tm, CACHE_QUEUE_LEN).await;

    let start_instant = Instant::now();

    for i in 0..times {
        let send_data = format!("Hello world, {}", i);

        sender
            .send(BytesMut::from(send_data.as_bytes()))
            .await
            .unwrap();

        let buf = receiver.next().await.unwrap();

        assert_eq!(&buf, send_data.as_bytes());
    }

    println!(
        "\t echo_single_client {:?}",
        start_instant.elapsed() / times as u32
    );
}
