use futures::{channel::mpsc::channel, AsyncReadExt, AsyncWriteExt, SinkExt, StreamExt};
use hala_io_util::*;
use hala_net::{
    quic::{
        Config, InnerConnector, QuicAcceptor, QuicConn, QuicConnector, QuicListener, QuicStream,
    },
    *,
};

const MAX_DATAGRAM_SIZE: usize = 1350;

use std::{io, net::SocketAddr, path::Path, sync::Arc, task::Poll};

use futures::{future::poll_fn, lock::Mutex, FutureExt};

use quiche::RecvInfo;

fn config(is_server: bool) -> Config {
    let mut config = Config::new().unwrap();

    config.verify_peer(false);

    if is_server {
        let root_path = Path::new(env!("CARGO_MANIFEST_DIR"));

        log::debug!("test run dir {:?}", root_path);

        config
            .load_cert_chain_from_pem_file(root_path.join("cert/cert.crt").to_str().unwrap())
            .unwrap();

        config
            .load_priv_key_from_pem_file(root_path.join("cert/cert.key").to_str().unwrap())
            .unwrap();
    }

    config
        .set_application_protos(&[b"hq-interop", b"hq-29", b"hq-28", b"hq-27", b"http/0.9"])
        .unwrap();

    config.set_max_idle_timeout(2000);
    config.set_max_recv_udp_payload_size(MAX_DATAGRAM_SIZE);
    config.set_max_send_udp_payload_size(MAX_DATAGRAM_SIZE);
    config.set_initial_max_data(10_000_000);
    config.set_initial_max_stream_data_bidi_local(1_000_000);
    config.set_initial_max_stream_data_bidi_remote(1_000_000);
    config.set_initial_max_streams_bidi(100);
    config.set_initial_max_streams_uni(100);
    config.set_disable_active_migration(false);

    config
}

#[test]
fn test_quic_connect_accept() {
    let laddr = "127.0.0.1:10234".parse().unwrap();
    let raddr = "127.0.0.1:20234".parse().unwrap();

    let mut connector = InnerConnector::new(&mut config(false), laddr, raddr).unwrap();

    let mut acceptor = QuicAcceptor::new(config(true)).unwrap();

    loop {
        let mut buf = [0; MAX_DATAGRAM_SIZE];

        let (send_size, send_info) = connector.send(&mut buf).unwrap();

        assert_eq!(send_info.from, laddr);
        assert_eq!(send_info.to, raddr);

        let (read_size, _) = acceptor
            .recv(
                &mut buf[..send_size],
                RecvInfo {
                    from: laddr,
                    to: raddr,
                },
            )
            .unwrap();

        assert_eq!(read_size, send_size);

        let (send_size, send_info) = acceptor.send(&mut buf).unwrap();

        assert_eq!(send_info.from, raddr);
        assert_eq!(send_info.to, laddr);

        if !acceptor.pop_established().is_empty() {
            assert!(connector.is_established());
            break;
        }

        let read_size = connector
            .recv(
                &mut buf[..send_size],
                RecvInfo {
                    from: raddr,
                    to: laddr,
                },
            )
            .unwrap();

        assert_eq!(read_size, send_size);
    }
}

#[allow(unused)]
async fn create_listener(ports: i32) -> (QuicListener, Vec<SocketAddr>) {
    let laddrs = (0..ports)
        .clone()
        .into_iter()
        .map(|_| "127.0.0.1:0".parse::<SocketAddr>().unwrap())
        .collect::<Vec<_>>();

    let listener = QuicListener::bind(laddrs.as_slice(), config(true)).unwrap();

    let laddrs = listener.local_addrs().map(|addr| *addr).collect::<Vec<_>>();

    (listener, laddrs)
}

#[hala_test::test(local_io_test)]
async fn test_async_quic() {
    let (mut listener, laddrs) = create_listener(10).await;

    local_io_spawn(async move {
        let mut connector = QuicConnector::bind("127.0.0.1:0", config(false)).unwrap();

        let conn = connector.connect(laddrs.as_slice()).await.unwrap();

        log::debug!("client connected !!!");

        let stream = conn.accept().await.unwrap();

        log::debug!("client accept one stream({:?})", stream);

        stream.stream_send(b"hello world", true).await.unwrap();

        Ok(())
    })
    .unwrap();

    let conn = listener.accept().await.unwrap();

    log::debug!("Accept one incoming conn({:?})", conn);

    let mut stream = conn.open_stream().await.unwrap();

    stream.write(b"hello").await.unwrap();

    let mut buf = [0; MAX_DATAGRAM_SIZE];

    let read_size = stream.read(&mut buf).await.unwrap();

    assert_eq!(&buf[..read_size], b"hello world");

    assert!(stream.is_closed().await);
}

#[hala_test::test(local_io_test)]
async fn test_connector_timeout() {
    let mut connector = QuicConnector::bind("127.0.0.1:0", config(false)).unwrap();

    let err = connector.connect("127.0.0.1:1812").await.unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::TimedOut);
}

#[hala_test::test(local_io_test)]
async fn test_conn_timeout() {
    _ = pretty_env_logger::try_init_timed();

    let (mut listener, laddrs) = create_listener(1).await;

    let mut connector = QuicConnector::bind("127.0.0.1:0", config(false)).unwrap();

    local_io_spawn(async move {
        let _ = listener.accept().await.unwrap();

        log::trace!("Accept one incoming");

        Ok(())
    })
    .unwrap();

    log::trace!("Client connect");

    let conn = connector.connect(laddrs.as_slice()).await.unwrap();

    log::trace!("Client connected");

    let mut stream = conn.open_stream().await.unwrap();

    stream.write(b"hello world").await.unwrap();

    let mut buf = [0; MAX_DATAGRAM_SIZE];

    log::trace!("Start stream recv");

    let error = stream.read(&mut buf).await.unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
}

async fn accept_stream(conn: QuicConn) -> io::Result<()> {
    while let Some(stream) = conn.accept().await {
        local_io_spawn(stream_echo(stream))?;
    }

    Ok(())
}

async fn stream_echo(mut stream: QuicStream) -> io::Result<()> {
    loop {
        let mut buf = vec![0; 65535];

        let read_size = stream.read(&mut buf).await?;

        stream.write_all(&buf[..read_size]).await?;
    }
}

#[hala_test::test(local_io_test)]
async fn test_multi_quic_stream() {
    _ = pretty_env_logger::try_init_timed();

    let (mut listener, laddrs) = create_listener(1).await;

    let mut connector = QuicConnector::bind("127.0.0.1:0", config(false)).unwrap();

    local_io_spawn(async move {
        let conn = listener.accept().await.unwrap();

        local_io_spawn(accept_stream(conn))?;

        Ok(())
    })
    .unwrap();

    let conn = connector.connect(laddrs.as_slice()).await.unwrap();

    let (s, mut r) = channel::<()>(0);

    let count = 2;

    for i in 0..count {
        let mut s = s.clone();

        let mut stream = conn.open_stream().await.unwrap();

        local_io_spawn(async move {
            let data: String = format!("hello world {}", i);

            stream.write(data.as_bytes()).await.unwrap();

            let mut buf = [0; 1024];

            let read_size = stream.read(&mut buf).await.unwrap();

            assert_eq!(&buf[..read_size], data.as_bytes());

            s.send(()).await.unwrap();

            log::trace!("stream {} complete", i);

            Ok(())
        })
        .unwrap();
    }

    for _ in 0..count {
        r.next().await.unwrap();
    }
}

#[hala_test::test(io_test)]
async fn test_lock() {
    let state = Arc::new(Mutex::new(1));

    poll_fn(|cx| -> Poll<()> {
        let state_cloned = state.clone();

        let try_lock = async move {
            let mut state = state_cloned.lock().await;

            log::debug!("state entry");

            poll_fn(|_| -> Poll<()> {
                *state = 2;
                std::task::Poll::Pending
            })
            .await;

            log::debug!("state leave");
        };

        let mut try_lock = Box::pin(try_lock);

        assert_eq!(try_lock.poll_unpin(cx), Poll::Pending);

        assert_eq!(try_lock.poll_unpin(cx), Poll::Pending);

        log::trace!("state outside entry");

        assert!(state.lock().poll_unpin(cx).is_pending());

        Poll::Ready(())
    })
    .await;
}

#[hala_test::test(io_test)]
async fn tcp_echo_test() {
    let echo_data = b"hello";

    let tcp_listener = TcpListener::bind("127.0.0.1:0").unwrap();

    let laddr = tcp_listener.local_addr().unwrap();

    for _ in 0..10 {
        io_spawn(async move {
            let tcp_stream = TcpStream::connect(&[laddr].as_slice()).unwrap();

            let mut buf = [0; 1024];

            let read_size = (&tcp_stream).read(&mut buf).await.unwrap();

            assert_eq!(read_size, echo_data.len());

            let write_size = (&tcp_stream).write(&buf[..read_size]).await.unwrap();

            assert_eq!(write_size, echo_data.len());

            Ok(())
        })
        .unwrap();

        let (conn, _) = tcp_listener.accept().await.unwrap();

        let write_size = (&conn).write(echo_data).await.unwrap();

        assert_eq!(write_size, echo_data.len());

        let mut buf = [0; 1024];

        let read_size = (&conn).read(&mut buf).await.unwrap();

        assert_eq!(read_size, echo_data.len());
    }
}

#[hala_test::test(io_test)]
async fn udp_echo_test() {
    let echo_data = b"hello";

    let udp_server = UdpSocket::bind("127.0.0.1:0").unwrap();

    let laddr = udp_server.local_addr().unwrap();

    for _ in 0..10 {
        io_spawn(async move {
            let udp_client = UdpSocket::bind("127.0.0.1:0").unwrap();

            let mut buf = [0; 1024];

            let write_size = udp_client.send_to(echo_data, laddr).await.unwrap();

            assert_eq!(write_size, echo_data.len());

            let (read_size, raddr) = udp_client.recv_from(&mut buf).await.unwrap();

            assert_eq!(read_size, echo_data.len());

            assert_eq!(raddr, laddr);

            Ok(())
        })
        .unwrap();

        let mut buf = [0; 1024];

        let (read_size, raddr) = udp_server.recv_from(&mut buf).await.unwrap();

        assert_eq!(read_size, echo_data.len());

        let write_size = udp_server.send_to(&buf[..read_size], raddr).await.unwrap();

        assert_eq!(write_size, echo_data.len());
    }
}
