use quiche::ConnectionId;

const MAX_DATAGRAM_SIZE: usize = 1350;

#[test]
fn test_stream() {
    _ = pretty_env_logger::try_init();

    let mut config = quiche::Config::new(quiche::PROTOCOL_VERSION).unwrap();
    config.set_application_protos(&[b"example-proto"]).unwrap();
    config.verify_peer(false);

    let scid = ConnectionId::from_ref(b"client");

    let local = "127.0.0.1:1812".parse().unwrap();
    let peer = "127.0.0.1:1813".parse().unwrap();

    let mut client_conn = quiche::connect(None, &scid, local, peer, &mut config).unwrap();

    assert!(
        !client_conn.is_established(),
        "client connection state is disconnected"
    );

    let mut server_conn = quiche::accept(&scid, None, peer, local, &mut config).unwrap();

    assert!(
        !server_conn.is_established(),
        "server connection state is disconnected"
    );

    let mut out = [0; MAX_DATAGRAM_SIZE];

    let (write_size, send_info) = client_conn.send(&mut out).unwrap();

    log::info!("client conn {:?} size {}", send_info, write_size);

    let recv_info = quiche::RecvInfo {
        to: send_info.to,
        from: send_info.from,
    };

    let read_size = server_conn.recv(&mut out[..write_size], recv_info).unwrap();

    assert_eq!(read_size, write_size);
}
