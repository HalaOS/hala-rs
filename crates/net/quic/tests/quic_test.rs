use std::{io, time::Duration};

use futures::{AsyncReadExt, AsyncWriteExt};
use hala_future::executor::future_spawn;
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
async fn test_establish() -> io::Result<()> {
    let listener = QuicListener::bind("127.0.0.1:0", mock_config(true, 1350)).unwrap();

    let raddr = listener.local_addr();

    future_spawn(async move {
        let _ = listener.accept().await.unwrap();
    });

    let _ = QuicConn::connect("127.0.0.1:0", raddr, &mut mock_config(false, 1350))
        .await
        .unwrap();

    Ok(())
}

#[hala_test::test(io_test)]
async fn test_connect_timeout() -> io::Result<()> {
    let mut config = mock_config(false, 1350);

    config.set_max_idle_timeout(1000);

    let error = QuicConn::connect("127.0.0.1:0", "127.0.0.1:1812", &mut config)
        .await
        .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::TimedOut);

    Ok(())
}

#[hala_test::test(io_test)]
async fn test_open_client_stream() -> io::Result<()> {
    let listener = QuicListener::bind("127.0.0.1:0", mock_config(true, 1350)).unwrap();

    let raddr = listener.local_addr();

    let send_data = b"test test a";

    future_spawn(async move {
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

    Ok(())
}

#[hala_test::test(io_test)]
async fn test_open_server_stream() -> io::Result<()> {
    let listener = QuicListener::bind("127.0.0.1:0", mock_config(true, 1350)).unwrap();

    let raddr = listener.local_addr();

    let send_data = b"hello hala os";

    future_spawn(async move {
        let conn = listener.accept().await.unwrap();

        let mut stream = conn.open_stream().await.unwrap();

        stream.write(send_data).await.unwrap();

        // wait client stream recv data.
        sleep(Duration::from_secs(1)).await.unwrap();
    });

    let conn = QuicConn::connect("127.0.0.1:0", raddr, &mut mock_config(false, 1350))
        .await
        .unwrap();

    let mut stream = conn.accept_stream().await.unwrap();

    let mut buf = vec![0; 1024];

    let read_size = stream.read(&mut buf).await.unwrap();

    assert_eq!(send_data, &buf[..read_size]);

    Ok(())
}

#[hala_test::test(io_test)]
async fn test_server_max_streams_stream() -> io::Result<()> {
    let mut config = mock_config(true, 1350);

    config.set_initial_max_streams_bidi(3);

    let listener = QuicListener::bind("127.0.0.1:0", config).unwrap();

    let raddr = listener.local_addr();

    let send_data = b"hello hala os";

    future_spawn(async move {
        let conn = listener.accept().await.unwrap();

        while let Some(_) = conn.accept_stream().await {}
    });

    let mut config = mock_config(false, 1350);

    let conn = QuicConn::connect("127.0.0.1:0", raddr, &mut config)
        .await
        .unwrap();

    {
        let quiche_conn = conn.to_quiche_conn().await;

        assert_eq!(quiche_conn.peer_streams_left_bidi(), 3);
    }

    let mut stream = conn.open_stream().await.unwrap();

    stream.write(send_data).await.unwrap();

    {
        let quiche_conn = conn.to_quiche_conn().await;

        assert_eq!(quiche_conn.peer_streams_left_bidi(), 1);
    }

    let mut stream = conn.open_stream().await.unwrap();

    stream.write(send_data).await.unwrap();

    let mut stream = conn.open_stream().await.unwrap();

    stream.write(send_data).await.expect_err("StreamLimit");

    Ok(())
}

#[hala_test::test(io_test)]
async fn test_client_max_streams_stream() -> io::Result<()> {
    let config = mock_config(true, 1350);

    let listener = QuicListener::bind("127.0.0.1:0", config).unwrap();

    let raddr = listener.local_addr();

    let send_data = b"hello hala os";

    future_spawn(async move {
        let mut config = mock_config(false, 1350);

        config.set_initial_max_streams_bidi(2);

        let conn = QuicConn::connect("127.0.0.1:0", raddr, &mut config)
            .await
            .unwrap();

        while let Some(_) = conn.accept_stream().await {}
    });

    let conn = listener.accept().await.unwrap();

    {
        let quiche_conn = conn.to_quiche_conn().await;

        assert_eq!(quiche_conn.peer_streams_left_bidi(), 2);
    }

    let mut stream = conn.open_stream().await.unwrap();

    stream.write(send_data).await.unwrap();

    {
        let quiche_conn = conn.to_quiche_conn().await;

        assert_eq!(quiche_conn.peer_streams_left_bidi(), 0);
    }

    let mut stream = conn.open_stream().await.unwrap();

    stream.write(send_data).await.expect_err("StreamLimit");

    Ok(())
}

#[hala_test::test(io_test)]
async fn test_dynamic_peer_streams_left_bid() -> io::Result<()> {
    _ = pretty_env_logger::try_init_timed();

    let mut config = mock_config(true, 1350);

    config.set_initial_max_streams_bidi(2);

    let listener = QuicListener::bind("127.0.0.1:0", config).unwrap();

    let raddr = listener.local_addr();

    let send_data = b"hello hala os";

    future_spawn(async move {
        let conn = listener.accept().await.unwrap();

        while let Some(mut stream) = conn.accept_stream().await {
            future_spawn(async move {
                let mut buf = vec![0; 1024];

                let read_size = stream.read(&mut buf).await.unwrap();

                stream
                    .stream_send(&mut buf[..read_size], true)
                    .await
                    .unwrap();

                // wait data write to client.
                // sleep(Duration::from_millis(100)).await.unwrap();

                log::info!("Server stream dropped");
            });
        }
    });

    let mut config = mock_config(false, 1350);

    config.set_initial_max_streams_bidi(2);

    let conn = QuicConn::connect("127.0.0.1:0", raddr, &mut config)
        .await
        .unwrap();

    let loops = 10;

    for _ in 0..loops {
        {
            let mut stream = conn.open_stream().await.unwrap();

            log::info!(
                "open stream, scid={:?}, dcid={:?}, stream_id={:?}",
                conn.source_id(),
                conn.destination_id(),
                stream.to_id()
            );

            stream.stream_send(send_data, false).await.unwrap();

            let mut buf = vec![0; 1024];

            log::info!("Client stream read");

            let read_size = stream.read(&mut buf).await.unwrap();

            assert_eq!(&buf[..read_size], send_data);
        }

        log::info!("Client stream dropped");

        loop {
            {
                let quiche_conn = conn.to_quiche_conn().await;

                assert!(!quiche_conn.is_closed());

                if quiche_conn.peer_streams_left_bidi() > 0 {
                    // log::info!("===========================");
                    break;
                }
            }

            log::info!("Client wait peer stream update");
            sleep(Duration::from_millis(500)).await.unwrap();
        }
    }

    Ok(())
}
