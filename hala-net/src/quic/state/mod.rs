mod conn;
mod connector;
mod listener;

pub use conn::*;
pub use connector::*;
pub use listener::*;

#[cfg(test)]
mod tests {

    use futures::FutureExt;
    use futures_test::task::noop_context;
    use hala_io_util::io_test;
    use quiche::RecvInfo;
    use std::task::Poll;

    use crate::quic::mock_config;

    use super::{QuicConnState, QuicConnectorState, QuicListenerState, QuicListenerStateRecv};

    struct MockQuic {
        #[allow(dead_code)]
        client: QuicConnState,
        #[allow(dead_code)]
        server_conn: QuicConnState,
        #[allow(dead_code)]
        listener: QuicListenerState,
    }

    const MAX_DATAGRAM_SIZE: usize = 1350;

    impl MockQuic {
        async fn new() -> MockQuic {
            let laddr = "127.0.0.1:1812".parse().unwrap();
            let raddr = "127.0.0.1:1813".parse().unwrap();

            let mut connector =
                QuicConnectorState::new(&mut mock_config(false, MAX_DATAGRAM_SIZE), laddr, raddr)
                    .unwrap();

            let listener = QuicListenerState::new(mock_config(true, MAX_DATAGRAM_SIZE)).unwrap();

            let mut buf = vec![0; 65535];

            let mut server_conn = None;

            loop {
                let (send_size, send_info) = connector.send(&mut buf).unwrap().unwrap();

                let recv = listener
                    .recv(
                        &mut buf[..send_size],
                        RecvInfo {
                            from: send_info.from,
                            to: send_info.to,
                        },
                    )
                    .await
                    .unwrap();

                let (send_size, send_info) = match recv {
                    QuicListenerStateRecv::WriteSize(write_size) => {
                        assert_eq!(write_size, send_size);

                        if server_conn.is_none() {
                            server_conn = Some(listener.accept().await.unwrap());
                        }

                        server_conn.as_ref().unwrap().read(&mut buf).await.unwrap()
                    }
                    QuicListenerStateRecv::Handshake(write_size, handshake) => {
                        assert_eq!(write_size, send_size);

                        handshake.send(&mut buf).unwrap().unwrap()
                    }
                };

                assert_eq!(send_info.from, raddr);
                assert_eq!(send_info.to, laddr);

                connector
                    .recv(
                        &mut buf[..send_size],
                        RecvInfo {
                            from: send_info.from,
                            to: send_info.to,
                        },
                    )
                    .unwrap();

                if connector.is_established() {
                    break;
                }
            }

            MockQuic {
                client: connector.into(),
                server_conn: server_conn.unwrap(),
                listener,
            }
        }
    }

    #[hala_test::test(io_test)]
    async fn test_connect() {
        let _mock = MockQuic::new().await;
    }

    #[hala_test::test(io_test)]
    async fn test_open_stream() {
        let mock = MockQuic::new().await;

        let stream_id = mock.client.open_stream().await.unwrap();

        assert_eq!(stream_id, 4);

        let stream_id = mock.client.open_stream().await.unwrap();

        assert_eq!(stream_id, 8);

        let stream_id = mock.server_conn.open_stream().await.unwrap();

        assert_eq!(stream_id, 5);

        let stream_id = mock.server_conn.open_stream().await.unwrap();

        assert_eq!(stream_id, 9);
    }

    #[hala_test::test(io_test)]
    async fn test_stream_write() {
        let mock = MockQuic::new().await;

        let stream_id = mock.client.open_stream().await.unwrap();

        let mut cx = noop_context();

        let send_buf = b"hello world";

        let result = Box::pin(mock.client.stream_write(stream_id, send_buf, true))
            .poll_unpin(&mut cx)
            .map(|len| len.expect("stream_write"));

        assert_eq!(result, Poll::Ready(send_buf.len()));
    }

    #[hala_test::test(io_test)]
    async fn test_stream_write_overflow() {
        let mock = MockQuic::new().await;

        let stream_id = mock.client.open_stream().await.unwrap();

        let mut cx = noop_context();

        let send_buf = &[0; MAX_DATAGRAM_SIZE];

        for _ in 0..10 {
            let result = Box::pin(mock.client.stream_write(stream_id, send_buf, true))
                .poll_unpin(&mut cx)
                .map(|len| len.expect("stream_write"));

            assert_eq!(result, Poll::Ready(MAX_DATAGRAM_SIZE));
        }

        let result = Box::pin(mock.client.stream_write(stream_id, send_buf, true))
            .poll_unpin(&mut cx)
            .map(|len| len.expect("stream_write"));

        assert_eq!(result, Poll::Pending);
    }
}
