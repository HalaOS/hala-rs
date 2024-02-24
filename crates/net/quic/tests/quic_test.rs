use std::{io, time::Duration};

use futures::{AsyncReadExt, AsyncWriteExt};
use hala_future::executor::future_spawn;
use hala_io::{sleep, test::io_test};
use hala_quic::{Config, QuicConn, QuicConnPool, QuicListener};

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

    config.set_max_idle_timeout(15000);
    config.set_max_recv_udp_payload_size(max_datagram_size);
    config.set_max_send_udp_payload_size(max_datagram_size);
    config.set_initial_max_data(10_000_000);
    config.set_initial_max_stream_data_bidi_local((max_datagram_size * 10) as u64);
    config.set_initial_max_stream_data_bidi_remote((max_datagram_size * 10) as u64);
    config.set_initial_max_streams_bidi(9);
    config.set_initial_max_streams_uni(9);
    config.set_disable_active_migration(false);
    config.set_active_connection_id_limit(100);

    config
}

#[hala_test::test(io_test)]
async fn test_establish() {
    let listener = QuicListener::bind("127.0.0.1:0", mock_config(true, 1350))
        .await
        .unwrap();

    let raddr = listener.local_addrs().next().unwrap().clone();

    future_spawn(async move {
        let conn = listener.accept().await.unwrap();

        future_spawn(async move {
            if let Some(mut stream) = conn.accept_stream().await {
                let mut buf = vec![0; 1370];
                stream.read(&mut buf).await.unwrap();
            }

            conn.close().await.unwrap();
        });
    });

    let mut conns = vec![];

    let mut config = mock_config(false, 1350);

    for _ in 0..80 {
        let conn = QuicConn::connect("127.0.0.1:0", raddr, &mut config)
            .await
            .unwrap();

        let mut stream = conn.open_stream().await.unwrap();

        stream.write(b"hello world").await.unwrap();

        conns.push(conn);
    }
}

#[hala_test::test(io_test)]
async fn test_connect_timeout() {
    let mut config = mock_config(false, 1350);

    config.set_max_idle_timeout(1000);

    let error = QuicConn::connect("127.0.0.1:0", "127.0.0.1:1812", &mut config)
        .await
        .unwrap_err();

    #[cfg(not(target_env = "msvc"))]
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);

    #[cfg(target_env = "msvc")]
    assert_eq!(error.kind(), io::ErrorKind::ConnectionReset);
}

#[hala_test::test(io_test)]
async fn test_open_client_stream() {
    let listener = QuicListener::bind("127.0.0.1:0", mock_config(true, 1350))
        .await
        .unwrap();

    let raddr = listener.local_addrs().next().unwrap().clone();

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
}

#[hala_test::test(io_test)]
async fn test_open_server_stream() {
    let listener = QuicListener::bind("127.0.0.1:0", mock_config(true, 1350))
        .await
        .unwrap();

    let raddr = listener.local_addrs().next().unwrap().clone();

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
}

#[hala_test::test(io_test)]
async fn test_server_max_streams_stream() {
    let mut config = mock_config(true, 1350);

    config.set_initial_max_streams_bidi(3);

    let listener = QuicListener::bind("127.0.0.1:0", config).await.unwrap();

    let raddr = listener.local_addrs().next().unwrap().clone();

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
}

#[hala_test::test(io_test)]
async fn test_client_max_streams_stream() {
    let config = mock_config(true, 1350);

    let listener = QuicListener::bind("127.0.0.1:0", config).await.unwrap();

    let raddr = listener.local_addrs().next().unwrap().clone();

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
}

#[hala_test::test(io_test)]
async fn test_dynamic_peer_streams_left_bid() {
    // _ = pretty_env_logger::try_init_timed();

    let mut config = mock_config(true, 1350);

    config.set_initial_max_streams_bidi(2);

    let listener = QuicListener::bind("127.0.0.1:0", config).await.unwrap();

    let raddr = listener.local_addrs().next().unwrap().clone();

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
}

#[hala_test::test(io_test)]
async fn test_conn_pool() {
    let mut config = mock_config(true, 1350);

    config.set_initial_max_streams_bidi(2);

    let listener = QuicListener::bind("127.0.0.1:0", config).await.unwrap();

    let raddr = listener.local_addrs().next().unwrap().clone();

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
            });
        }
    });

    let conn_pool = QuicConnPool::new(10, raddr, mock_config(false, 1350)).unwrap();

    let mut streams = vec![];

    for _ in 0..10 {
        let stream = conn_pool.open_stream().await.unwrap();

        // stream.write(b"hello").await.unwrap();

        streams.push(stream);
    }

    let err = conn_pool.open_stream().await.expect_err("WouldBlock");

    assert_eq!(err.kind(), io::ErrorKind::WouldBlock);

    streams.drain(..);

    loop {
        // waiting stream closed.
        sleep(Duration::from_secs(1)).await.unwrap();

        if conn_pool.open_stream().await.is_ok() {
            break;
        }
    }
}

#[hala_test::test(io_test)]
async fn test_conn_pool_reconnect() {
    // pretty_env_logger::init_timed();

    let mut config = mock_config(true, 1350);

    config.set_initial_max_streams_bidi(2);

    let listener = QuicListener::bind("127.0.0.1:0", config).await.unwrap();

    let raddr = listener.local_addrs().next().unwrap().clone();

    future_spawn(async move {
        while let Some(conn) = listener.accept().await {
            conn.close().await.unwrap();
        }
    });

    let conn_pool = QuicConnPool::new(1, raddr, mock_config(false, 1350)).unwrap();

    for i in 0..10 {
        log::trace!("open stream {}", i);

        let mut stream = conn_pool.open_stream().await.expect("Reconnect");

        stream.write(b"hello").await.unwrap();

        loop {
            if let Err(err) = stream.write(b"hello").await {
                assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
                break;
            }

            sleep(Duration::from_millis(10)).await.unwrap();
        }
    }
}
