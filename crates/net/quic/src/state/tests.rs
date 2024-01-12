use futures::FutureExt;
use futures_test::task::noop_context;
use hala_future::poll_once;
use hala_io::test::io_test;
use quiche::RecvInfo;
use std::{io, net::SocketAddr, task::Poll};

use crate::mock_config;

use super::{QuicConnState, QuicConnectorState, QuicListenerState, QuicListenerWriteResult};

struct MockQuic {
    #[allow(dead_code)]
    client: QuicConnState,
    #[allow(dead_code)]
    server_conn: Option<QuicConnState>,
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
                .write(
                    &mut buf,
                    send_size,
                    RecvInfo {
                        from: send_info.from,
                        to: send_info.to,
                    },
                )
                .await
                .unwrap();

            let (send_size, send_info) = match recv {
                QuicListenerWriteResult::WriteSize(_) => {
                    panic!("not here");
                }
                QuicListenerWriteResult::Internal {
                    write_size,
                    read_size,
                    send_info,
                } => {
                    assert_eq!(write_size, send_size);

                    (read_size, send_info)
                }
                QuicListenerWriteResult::Incoming {
                    conn,
                    write_size,
                    read_size,
                    send_info,
                } => {
                    assert_eq!(write_size, send_size);

                    server_conn = Some(conn);

                    (read_size, send_info)
                }
            };

            assert_eq!(send_info.from, raddr);
            assert_eq!(send_info.to, laddr);

            if send_size != 0 {
                connector
                    .recv(
                        &mut buf[..send_size],
                        RecvInfo {
                            from: send_info.from,
                            to: send_info.to,
                        },
                    )
                    .unwrap();
            }

            if connector.is_established() {
                break;
            }
        }

        MockQuic {
            client: connector.into(),
            server_conn,
            listener,
        }
    }
}

impl MockQuic {
    async fn send_to_server(&mut self) -> io::Result<()> {
        let mut buf = vec![0; 65535];

        let (read_size, send_info) = self.client.read(&mut buf).await?;

        let result = self
            .listener
            .write(
                &mut buf,
                read_size,
                RecvInfo {
                    from: send_info.from,
                    to: send_info.to,
                },
            )
            .await?;

        match result {
            QuicListenerWriteResult::WriteSize(write_size) => assert_eq!(write_size, read_size),
            QuicListenerWriteResult::Internal {
                write_size: _,
                read_size: _,
                send_info: _,
            } => {
                panic!("not here");
            }
            QuicListenerWriteResult::Incoming {
                conn,
                write_size: _,
                read_size,
                send_info,
            } => {
                assert!(self.server_conn.is_none());

                self.server_conn = Some(conn);

                self.client
                    .write(
                        &mut buf[..read_size],
                        RecvInfo {
                            from: send_info.from,
                            to: send_info.to,
                        },
                    )
                    .await?;
            }
        }

        Ok(())
    }

    async fn send_to_client(&self) -> io::Result<()> {
        let mut buf = vec![0; 65535];

        let (read_size, send_info) = self.server_conn.as_ref().unwrap().read(&mut buf).await?;

        self.client
            .write(
                &mut buf[..read_size],
                RecvInfo {
                    from: send_info.from,
                    to: send_info.to,
                },
            )
            .await?;

        Ok(())
    }
}

#[hala_test::test(io_test)]
async fn test_connect() -> io::Result<()> {
    let mut mock = MockQuic::new().await;

    assert!(mock.server_conn.is_none());

    let _ = mock.client.open_stream().await.unwrap();

    mock.send_to_server().await.unwrap();

    assert!(mock.server_conn.is_some());

    Ok(())
}

#[hala_test::test(io_test)]
async fn test_open_stream() -> io::Result<()> {
    let mut mock = MockQuic::new().await;

    let stream_id = mock.client.open_stream().await.unwrap();

    assert_eq!(stream_id, 4);

    let stream_id = mock.client.open_stream().await.unwrap();

    assert_eq!(stream_id, 8);

    mock.send_to_server().await.unwrap();

    let stream_id = mock
        .server_conn
        .as_ref()
        .unwrap()
        .open_stream()
        .await
        .unwrap();

    assert_eq!(stream_id, 5);

    let stream_id = mock
        .server_conn
        .as_ref()
        .unwrap()
        .open_stream()
        .await
        .unwrap();

    assert_eq!(stream_id, 9);

    Ok(())
}

#[hala_test::test(io_test)]
async fn test_stream_write() -> io::Result<()> {
    let mock = MockQuic::new().await;

    let stream_id = mock.client.open_stream().await.unwrap();

    let mut cx = noop_context();

    let send_buf = b"hello world";

    let result = Box::pin(mock.client.stream_send(stream_id, send_buf, true))
        .poll_unpin(&mut cx)
        .map(|len| len.expect("stream_write"));

    assert_eq!(result, Poll::Ready(send_buf.len()));

    Ok(())
}

#[hala_test::test(io_test)]
async fn test_max_stream_data() -> io::Result<()> {
    let mock = MockQuic::new().await;

    let stream_id = mock.client.open_stream().await.unwrap();

    let send_buf = &[0; MAX_DATAGRAM_SIZE];

    for _ in 0..10 {
        let result = poll_once!(mock.client.stream_send(stream_id, send_buf, false))
            .map(|len| len.expect(""));

        assert_eq!(result, Poll::Ready(MAX_DATAGRAM_SIZE));
    }

    let result =
        poll_once!(mock.client.stream_send(stream_id, send_buf, false)).map(|len| len.expect(""));

    assert_eq!(result, Poll::Pending);

    Ok(())
}

#[hala_test::test(io_test)]
async fn test_server_stream_accept() -> io::Result<()> {
    let send_data = b"hello";

    let mut mock = MockQuic::new().await;

    let client_to_stream_id = mock.client.open_stream().await.unwrap();

    mock.client
        .stream_send(client_to_stream_id, send_data, true)
        .await
        .unwrap();

    mock.send_to_server().await.unwrap();

    let server_conn = mock.server_conn.as_ref().unwrap();

    let server_from_stream_id = server_conn.accept().await;

    assert_eq!(server_from_stream_id, Some(client_to_stream_id));

    let mut buf = vec![0; 1024];

    let (read_size, fin) = server_conn
        .stream_recv(server_from_stream_id.unwrap(), &mut buf)
        .await
        .unwrap();

    assert_eq!(read_size, send_data.len());

    assert_eq!(&buf[..read_size], send_data);

    assert!(fin);

    let send_data2 = b"hello world";

    server_conn
        .stream_send(client_to_stream_id, send_data2, false)
        .await
        .unwrap();

    mock.send_to_client().await.unwrap();

    mock.client
        .stream_recv(client_to_stream_id, &mut buf)
        .await
        .unwrap();

    assert_eq!(&buf[..read_size], send_data);

    Ok(())
}

#[hala_test::test(io_test)]
async fn test_client_stream_accept() -> io::Result<()> {
    let mut mock = MockQuic::new().await;

    let _ = mock.client.open_stream().await.unwrap();

    mock.send_to_server().await.unwrap();

    let server_conn = mock
        .server_conn
        .as_ref()
        .expect("Server connection established");

    let server_open_stream_id = server_conn.open_stream().await.unwrap();

    let server_send_data = b"hello world ............";

    let send_size = server_conn
        .stream_send(server_open_stream_id, server_send_data, false)
        .await
        .unwrap();

    assert_eq!(send_size, server_send_data.len());

    mock.send_to_client().await.unwrap();

    let client_accept_stream_id = mock
        .client
        .accept()
        .await
        .expect("Client connection dropped");

    assert_eq!(client_accept_stream_id, server_open_stream_id);

    let mut buf = vec![0; 1024];

    let (read_size, fin) = mock
        .client
        .stream_recv(client_accept_stream_id, &mut buf)
        .await
        .unwrap();

    assert!(!fin);

    assert_eq!(&buf[..read_size], server_send_data);

    Ok(())
}

#[hala_test::test(io_test)]
async fn test_stream_stopped() -> io::Result<()> {
    let mut mock = MockQuic::new().await;

    let stream_id = mock.client.open_stream().await.unwrap();

    mock.client
        .stream_send(stream_id, b"hello", true)
        .await
        .unwrap();

    mock.send_to_server().await.unwrap();

    let server_conn = mock
        .server_conn
        .as_ref()
        .expect("Server connection established");

    assert_eq!(
        server_conn.accept().await.expect("New incoming stream"),
        stream_id
    );

    server_conn.stream_shutdown(stream_id, 1).await.unwrap();

    mock.send_to_client().await.unwrap();

    mock.client
        .stream_send(stream_id, b"hello", true)
        .await
        .expect_err("Server shutdown stream");

    Ok(())
}

#[hala_test::test(io_test)]
async fn test_client_max_open_streams() -> io::Result<()> {
    let mut mock = MockQuic::new().await;

    // stream 1 reserved for crypto handshake.
    for _ in 0..8 {
        let stream_id = mock.client.open_stream().await.unwrap();

        mock.client
            .stream_send(stream_id, b"hello", true)
            .await
            .unwrap();

        mock.send_to_server().await.unwrap();
    }

    let stream_id = mock.client.open_stream().await.unwrap();

    mock.client
        .stream_send(stream_id, b"hello", true)
        .await
        .expect_err("Stream limits");

    Ok(())
}

#[hala_test::test(io_test)]
async fn test_server_max_open_streams() -> io::Result<()> {
    let mut mock = MockQuic::new().await;

    _ = mock.client.open_stream().await.unwrap();

    mock.send_to_server().await.unwrap();

    let server_conn = mock
        .server_conn
        .as_ref()
        .expect("Server connection established");

    // stream 1 reserved for crypto handshake.
    for _ in 0..8 {
        let stream_id = server_conn.open_stream().await.unwrap();

        server_conn
            .stream_send(stream_id, b"hello", true)
            .await
            .unwrap();

        mock.send_to_client().await.unwrap();
    }

    let stream_id = server_conn.open_stream().await.unwrap();

    server_conn
        .stream_send(stream_id, b"hello", true)
        .await
        .expect_err("Stream limits");

    Ok(())
}

#[hala_test::test(io_test)]
async fn test_multi_path() -> io::Result<()> {
    let mut mock = MockQuic::new().await;

    let stream_id = mock.client.open_stream().await.unwrap();

    mock.client
        .stream_send(stream_id, b"hello", false)
        .await
        .unwrap();

    mock.send_to_server().await.unwrap();

    let client_conn = mock.client.clone();

    let server_conn = mock
        .server_conn
        .clone()
        .expect("Server connection established");

    client_conn
        .stream_send(stream_id, b" world", false)
        .await
        .unwrap();

    let mut read_buf = vec![0; 2000];

    let (read_size, send_info) = client_conn.read(&mut read_buf).await.unwrap();

    let new_from = "127.0.0.1:1024".parse::<SocketAddr>().unwrap();
    let new_to = "127.0.0.1:1023".parse::<SocketAddr>().unwrap();

    assert_ne!(new_from, send_info.from);
    assert_ne!(new_to, send_info.to);

    let recv_info = RecvInfo {
        from: new_from,
        to: new_to,
    };

    let _ = server_conn
        .write(&mut read_buf[..read_size], recv_info)
        .await
        .unwrap();

    let stream_id = server_conn.accept().await.unwrap();

    let (recv_size, fin) = server_conn
        .stream_recv(stream_id, &mut read_buf)
        .await
        .unwrap();

    assert!(!fin);

    assert_eq!(&read_buf[..recv_size], b"hello world");

    Ok(())
}

#[hala_test::test(io_test)]
async fn test_scids_left() -> io::Result<()> {
    // pretty_env_logger::init_timed();

    // use config.set_active_connection_id_limit to change active_connection_id_limit parameter.

    let mock = MockQuic::new().await;

    assert_eq!(mock.client.scids_left().await, 1);

    Ok(())
}

#[hala_test::test(io_test)]
async fn verify_client_cert() -> io::Result<()> {
    let mut mock = MockQuic::new().await;

    let stream_id = mock.client.open_stream().await.unwrap();

    mock.client
        .stream_send(stream_id, b"hello", false)
        .await
        .unwrap();

    mock.send_to_server().await.unwrap();

    let server_conn = mock
        .server_conn
        .clone()
        .expect("Server connection established");

    assert!(server_conn.to_quiche_conn().await.peer_cert().is_some());

    Ok(())
}

#[hala_test::test(io_test)]
async fn stream_normal_closed() -> io::Result<()> {
    let mut mock = MockQuic::new().await;

    let client_conn = mock.client.clone();

    let client_stream_id = client_conn.open_stream().await.unwrap();

    client_conn
        .stream_send(client_stream_id, b"hello", false)
        .await
        .unwrap();

    mock.send_to_server().await.unwrap();

    let server_conn = mock
        .server_conn
        .clone()
        .expect("Server connection established");

    let server_stream_id = server_conn.accept().await.unwrap();

    server_conn
        .stream_send(server_stream_id, b"", true)
        .await
        .unwrap();

    mock.send_to_client().await.unwrap();

    client_conn
        .stream_send(client_stream_id, b"", true)
        .await
        .unwrap();

    assert!(client_conn
        .to_quiche_conn()
        .await
        .stream_finished(client_stream_id));

    mock.send_to_server().await.unwrap();

    assert!(!server_conn
        .to_quiche_conn()
        .await
        .stream_finished(server_stream_id));

    Ok(())
}
