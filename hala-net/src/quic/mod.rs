mod acceptor;
use acceptor::*;

mod conn_state;
use conn_state::*;

mod stream;
pub use stream::*;

mod conn;
pub use conn::*;

mod connector;
pub use connector::*;

mod config;
pub use config::*;

mod listener;
pub use listener::*;

#[cfg(test)]
mod tests {

    const MAX_DATAGRAM_SIZE: usize = 1350;

    use std::{io, net::SocketAddr, path::Path, sync::Arc, task::Poll};

    use futures::{future::poll_fn, lock::Mutex, AsyncReadExt, AsyncWriteExt, FutureExt};
    use hala_io_util::io_spawn;
    use quiche::RecvInfo;

    use super::{acceptor::QuicAcceptor, *};

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

        config.set_max_idle_timeout(500);
        config.set_max_recv_udp_payload_size(MAX_DATAGRAM_SIZE);
        config.set_max_send_udp_payload_size(MAX_DATAGRAM_SIZE);
        config.set_initial_max_data(10_000_000);
        config.set_initial_max_stream_data_bidi_local(1_000_000);
        config.set_initial_max_stream_data_bidi_remote(1_000_000);
        config.set_initial_max_streams_bidi(100);
        config.set_initial_max_streams_uni(100);
        config.set_disable_active_migration(true);

        config
    }

    #[test]
    fn test_connect_accept() {
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

    #[hala_io_test::test]
    async fn test_async_quic() {
        let (mut listener, laddrs) = create_listener(10).await;

        io_spawn(async move {
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

    #[hala_io_test::test]
    async fn test_connector_timeout() {
        // pretty_env_logger::init_timed();
        let mut connector = QuicConnector::bind("127.0.0.1:0", config(false)).unwrap();

        let err = connector.connect("127.0.0.1:1812").await.unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }

    #[hala_io_test::test]
    async fn test_conn_timeout() {
        // pretty_env_logger::init_timed();

        let (mut listener, laddrs) = create_listener(1).await;

        let mut connector = QuicConnector::bind("127.0.0.1:0", config(false)).unwrap();

        io_spawn(async move {
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

        let error = stream.read(&mut buf).await.unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }

    #[hala_io_test::test]
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
}
