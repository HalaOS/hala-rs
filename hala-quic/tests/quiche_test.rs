#![allow(unused)]

use std::path::Path;

use quiche::{ConnectionId, Header, RecvInfo, Type, MAX_CONN_ID_LEN};
use rand::{rngs::OsRng, RngCore};

const MAX_DATAGRAM_SIZE: usize = 1350;

#[test]
fn test_stream() {
    _ = pretty_env_logger::try_init();

    let mut config = quiche::Config::new(quiche::PROTOCOL_VERSION).unwrap();
    config.set_application_protos(&[b"example-proto"]).unwrap();
    config.verify_peer(false);

    let root_path = Path::new(env!("CARGO_MANIFEST_DIR"));

    log::debug!("test run dir {:?}", root_path);

    config
        .load_cert_chain_from_pem_file(root_path.join("tests/cert.crt").to_str().unwrap())
        .unwrap();

    config
        .load_priv_key_from_pem_file(root_path.join("tests/cert.key").to_str().unwrap())
        .unwrap();

    config.set_max_idle_timeout(5000);
    config.set_max_recv_udp_payload_size(MAX_DATAGRAM_SIZE);
    config.set_max_send_udp_payload_size(MAX_DATAGRAM_SIZE);
    config.set_initial_max_data(10_000_000);
    config.set_initial_max_stream_data_bidi_local(1_000_000);
    config.set_initial_max_stream_data_bidi_remote(1_000_000);
    config.set_initial_max_streams_bidi(100);
    config.set_initial_max_streams_uni(100);
    config.set_disable_active_migration(true);

    let scid = ConnectionId::from_ref(b"client");

    let local = "127.0.0.1:1812".parse().unwrap();
    let peer = "127.0.0.1:1813".parse().unwrap();

    let mut client_conn = quiche::connect(None, &scid, local, peer, &mut config).unwrap();

    assert!(
        !client_conn.is_established(),
        "client connection state is disconnected"
    );

    let mut out = [0; MAX_DATAGRAM_SIZE];

    /// handshake.
    let (write_size, send_info) = client_conn.send(&mut out).unwrap();

    log::info!(
        "handshake len {}, client connection state: {:?}",
        write_size,
        client_conn.stats()
    );

    let header = Header::from_slice(&mut out[..write_size], MAX_CONN_ID_LEN).unwrap();

    log::info!("client handshake header: {:?}", header);

    assert_eq!(header.ty, Type::Initial);

    quiche::version_is_supported(header.version);

    assert!(header.token.unwrap().is_empty());

    let odcid = header.dcid.clone();

    let len = quiche::retry(
        &header.scid,
        &header.dcid,
        &ConnectionId::from_ref(b"server"),
        b"server token",
        header.version,
        &mut out,
    )
    .unwrap();

    let recv_info = RecvInfo {
        from: send_info.to,
        to: send_info.from,
    };

    /// client handle token
    let read_size = client_conn.recv(&mut out[..len], recv_info).unwrap();

    assert_eq!(read_size, len);

    let (write_size, send_info) = client_conn.send(&mut out).unwrap();

    log::info!(
        "handshake len {}, client connection state: {:?}",
        write_size,
        client_conn.stats()
    );

    let header = Header::from_slice(&mut out[..write_size], MAX_CONN_ID_LEN).unwrap();

    log::info!("client header with token: {:?}", header);

    assert_eq!(header.token.unwrap(), b"server token");

    let mut server_conn = quiche::accept(
        &ConnectionId::from_ref(b"server"),
        Some(&odcid),
        peer,
        local,
        &mut config,
    )
    .unwrap();

    loop {
        let recv_info = RecvInfo {
            from: send_info.from,
            to: send_info.to,
        };

        let read_size = server_conn.recv(&mut out[..write_size], recv_info).unwrap();

        log::info!(
            "{} processed {} bytes, state: {:?}",
            server_conn.trace_id(),
            read_size,
            server_conn.stats()
        );

        let (write_size, _) = server_conn.send(&mut out).unwrap();

        log::info!(
            "{} send {} bytes, state: {:?}",
            server_conn.trace_id(),
            write_size,
            server_conn.stats()
        );

        let recv_info = RecvInfo {
            from: send_info.to,
            to: send_info.from,
        };

        /// accept return
        let read_size = client_conn.recv(&mut out[..write_size], recv_info).unwrap();

        assert_eq!(read_size, write_size);

        let (write_size, send_info) = client_conn.send(&mut out).unwrap();

        log::info!(
            "accept echo len {}, client connection state: {:?},{}",
            write_size,
            client_conn.stats(),
            client_conn.is_established()
        );

        if client_conn.is_established() && server_conn.is_established() {
            break;
        }
    }

    for i in 0..10 {
        log::debug!("write stream data {}", i);
        assert!(client_conn.is_established());

        let stream_write_size = client_conn
            .stream_send(0x120, b"hello world", false)
            .unwrap();

        let (write_size, _) = client_conn.send(&mut out).unwrap();

        let recv_info = RecvInfo {
            from: send_info.from,
            to: send_info.to,
        };

        let read_size = server_conn.recv(&mut out[..write_size], recv_info).unwrap();

        assert_eq!(read_size, write_size);

        assert!(server_conn.stream_readable(0x120));

        let (stream_read_size, fin) = server_conn.stream_recv(0x120, &mut out).unwrap();

        assert_eq!(stream_read_size, stream_write_size);

        assert!(!fin);

        assert_eq!(&out[..stream_read_size], b"hello world");
    }
}
