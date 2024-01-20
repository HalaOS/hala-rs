use std::{io, net::SocketAddr, sync::OnceLock, time::Instant};

use futures::{
    channel::mpsc::channel, future::BoxFuture, AsyncReadExt, AsyncWriteExt, SinkExt, StreamExt,
};
use hala_future::executor::{block_on, future_spawn};
use hala_quic::{Config, QuicConn};
use hala_rproxy::{
    gateway::Gateway,
    handshake::{HandshakeContext, HandshakeResult, Handshaker},
    quic::QuicGateway,
    transport::{ChannelOpenFlag, Transport, TransportChannel, TransportManager},
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

struct MockHandshaker {}

impl Handshaker for MockHandshaker {
    fn handshake(
        &self,
        forward_cx: HandshakeContext,
    ) -> futures::prelude::future::BoxFuture<'static, io::Result<HandshakeResult>> {
        Box::pin(async move {
            Ok(HandshakeResult {
                context: forward_cx,
                transport_id: "EchoTransport".into(),
                channel_open_flag: ChannelOpenFlag::ConnectString("".into()),
            })
        })
    }
}

struct MockTransport {}

impl Transport for MockTransport {
    fn id(&self) -> &str {
        "EchoTransport"
    }

    fn open_channel(
        &self,
        _: ChannelOpenFlag,
        max_packet_len: usize,
        cache_queue_len: usize,
    ) -> BoxFuture<'static, io::Result<TransportChannel>> {
        let (forward_sender, mut forward_receiver) = channel(cache_queue_len);
        let (mut backward_sender, backward_receiver) = channel(cache_queue_len);

        future_spawn(async move {
            while let Some(buf) = forward_receiver.next().await {
                if backward_sender.send(buf).await.is_err() {
                    return;
                }
            }
        });

        Box::pin(async move {
            Ok(TransportChannel::new(
                max_packet_len,
                cache_queue_len,
                forward_sender,
                backward_receiver,
            ))
        })
    }
}

fn mock_tm() -> TransportManager {
    let tm = TransportManager::new(MockHandshaker {}, 1024, 1370);

    tm.register(MockTransport {});

    tm
}

async fn setup() -> SocketAddr {
    let gateway = QuicGateway::bind("hello", "127.0.0.1:0", mock_config(true, 1350)).unwrap();

    let quic_listener = gateway.listener.clone();

    std::thread::spawn(move || {
        gateway.start(mock_tm()).unwrap();
        gateway.join();
    });

    let raddr = *quic_listener.local_addrs().next().unwrap();

    raddr
}

const SERVER_ADDR: OnceLock<SocketAddr> = OnceLock::new();

fn main() {
    // pretty_env_logger::init_timed();
    let raddr = SERVER_ADDR
        .get_or_init(|| {
            let addr = block_on(setup(), 10);

            addr
        })
        .clone();

    divan::main();

    println!("quic_gateway");

    block_on(echo_single_client(raddr.clone(), 10000), 10);
}

async fn echo_single_client(raddr: SocketAddr, times: usize) {
    let conn = QuicConn::connect("127.0.0.1:0", raddr, &mut mock_config(false, 1350))
        .await
        .unwrap();

    let start_instant = Instant::now();

    let mut stream = conn.open_stream().await.unwrap();

    for i in 0..1000 {
        let data = format!("hello world {}", i);

        stream.write_all(data.as_bytes()).await.unwrap();

        let mut buf = vec![0; 1024];

        let read_size = stream.read(&mut buf).await.unwrap();

        assert_eq!(data.as_bytes(), &buf[..read_size]);
    }

    println!(
        "\t echo_single_client {:?}",
        start_instant.elapsed() / times as u32
    );
}
