use std::time::Duration;

use futures::{AsyncReadExt, AsyncWriteExt};
use hala_future::executor::spawn;
use hala_io::{sleep, test::io_test};
use hala_quic::{Config, QuicConn, QuicListener};

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

#[hala_test::test(io_test)]
async fn test_establish() {
    let listener = QuicListener::bind("127.0.0.1:0", mock_config(true, 1350)).unwrap();

    let raddr = listener.local_addr();

    spawn(async move {
        let _ = listener.accept().await.unwrap();
    });

    let _ = QuicConn::connect("127.0.0.1:0", raddr, &mut mock_config(false, 1350))
        .await
        .unwrap();
}

#[hala_test::test(io_test)]
async fn test_open_client_stream() {
    let listener = QuicListener::bind("127.0.0.1:0", mock_config(true, 1350)).unwrap();

    let raddr = listener.local_addr();

    let send_data = b"test test a";

    spawn(async move {
        let conn = listener.accept().await.unwrap();

        let mut stream = conn.accept_stream().await.unwrap();

        let mut buf = vec![0; 1024];

        let read_size = stream.read(&mut buf).await.unwrap();

        assert_eq!(&buf[..read_size], send_data);

        stream.write(&buf[..read_size]).await.unwrap();

        // wait client stream recv data.
        sleep(Duration::from_secs(1)).await.unwrap();
    });

    let conn = QuicConn::connect("127.0.0.1:0", raddr, &mut mock_config(false, 1350))
        .await
        .unwrap();

    let mut stream = conn.open_stream().await.unwrap();

    stream.write(send_data).await.unwrap();

    let mut buf = vec![0; 1024];

    let read_size = stream.read(&mut buf).await.unwrap();

    assert_eq!(&buf[..read_size], send_data);
}
