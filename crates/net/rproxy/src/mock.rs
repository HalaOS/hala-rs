use std::{io, net::SocketAddr, time::Duration};

use futures::AsyncWriteExt;
use hala_future::executor::future_spawn;
use hala_io::sleep;
use hala_quic::{QuicConn, QuicListener, QuicStream};

use crate::{
    make_tunnel_factory_channel, HandshakeContext, TransportConfig, TunnelFactoryManager,
    TunnelFactoryReceiver, TunnelOpenConfig,
};

pub(crate) fn mock_config(is_server: bool, max_datagram_size: usize) -> hala_quic::Config {
    use std::path::Path;

    let mut config = hala_quic::Config::new().unwrap();

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

pub(crate) fn create_quic_echo_server(max_streams_bidi: u64) -> QuicListener {
    let mut config = mock_config(true, 1370);

    config.set_initial_max_streams_bidi(max_streams_bidi);

    let listener = QuicListener::bind("127.0.0.1:0", config).unwrap();

    let listener_cloned = listener.clone();

    future_spawn(async move {
        while let Some(conn) = listener_cloned.accept().await {
            future_spawn(echo_handle_quic_conn(conn));
        }
    });

    listener
}

pub(crate) fn create_quic_conn_drop_server(
    max_streams_bidi: u64,
    timeout: Duration,
) -> QuicListener {
    let mut config = mock_config(true, 1370);

    config.set_initial_max_streams_bidi(max_streams_bidi);

    let listener = QuicListener::bind("127.0.0.1:0", config).unwrap();

    let listener_cloned = listener.clone();

    future_spawn(async move {
        while let Some(conn) = listener_cloned.accept().await {
            future_spawn(handle_quic_conn_timeout_drop(conn, timeout));
        }
    });

    listener
}

async fn handle_quic_conn_timeout_drop(conn: QuicConn, duration: Duration) {
    sleep(duration).await.unwrap();

    drop(conn);
}

async fn echo_handle_quic_conn(conn: QuicConn) {
    while let Some(stream) = conn.accept_stream().await {
        future_spawn(echo_handle_quic_stream(stream));
    }
}

async fn echo_handle_quic_stream(mut stream: QuicStream) {
    let mut buf = vec![0; 1370];

    loop {
        let (read_size, fin) = stream.stream_recv(&mut buf).await.unwrap();

        stream.write_all(&buf[..read_size]).await.unwrap();

        if fin {
            return;
        }
    }
}

pub(crate) fn tunnel_open_flag(tunnel_service_id: &str, raddr: SocketAddr) -> TunnelOpenConfig {
    TunnelOpenConfig {
        max_packet_len: 1370,
        max_cache_len: 10,
        tunnel_service_id: tunnel_service_id.into(),
        transport_config: TransportConfig::Quic(vec![raddr], mock_config(false, 1370)),
    }
}

/// Create new mock manager.
pub(crate) fn mock_tunnel_factory_manager() -> (TunnelFactoryManager, TunnelFactoryReceiver) {
    let (sender, receiver) = make_tunnel_factory_channel("MockTunnelFactory", 1024);

    let manager = TunnelFactoryManager::new(mock_handshaker);

    manager.register(sender);

    (manager, receiver)
}

pub(crate) async fn mock_handshaker(
    cx: HandshakeContext,
) -> io::Result<(HandshakeContext, TunnelOpenConfig)> {
    let config = TunnelOpenConfig {
        max_cache_len: 1024,
        max_packet_len: 1370,
        tunnel_service_id: "MockTunnelFactory".into(),
        transport_config: TransportConfig::None,
    };

    Ok((cx, config))
}
