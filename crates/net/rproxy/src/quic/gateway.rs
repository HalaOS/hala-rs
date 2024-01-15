use std::{io, net::ToSocketAddrs};

use hala_future::executor::future_spawn;
use hala_quic::{Config, QuicListener};

use crate::{gateway::Gateway, transport::TransportManager};

/// Gatway for Quic protocol
pub struct QuicGateway {
    listener: QuicListener,
    id: String,
    join_sender: std::sync::mpsc::Sender<()>,
    join_receiver: std::sync::mpsc::Receiver<()>,
}

impl QuicGateway {
    /// Create [`QuicGateway`] instance and bind quic server listener to `laddrs`.
    pub fn bind<ID: ToString, L: ToSocketAddrs>(
        id: ID,
        laddrs: L,
        config: Config,
    ) -> io::Result<Self> {
        let listener = QuicListener::bind(laddrs, config)?;

        let (join_sender, join_receiver) = std::sync::mpsc::channel();

        Ok(QuicGateway {
            listener,
            id: id.to_string(),
            join_sender,
            join_receiver,
        })
    }
}

impl Gateway for QuicGateway {
    fn start(&self, transport_manager: crate::transport::TransportManager) -> io::Result<()> {
        let join_sender = self.join_sender.clone();

        future_spawn(event_loop::run_loop(
            self.id.clone(),
            self.listener.clone(),
            join_sender,
            transport_manager,
        ));

        Ok(())
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn join(&self) {
        _ = self.join_receiver.recv();
    }

    fn stop(&self) -> io::Result<()> {
        let listenr = self.listener.clone();
        // close listener and drop incoming loop
        future_spawn(async move { listenr.close().await });

        Ok(())
    }
}

mod event_loop {
    use bytes::BytesMut;
    use futures::{
        channel::mpsc::{Receiver, Sender},
        AsyncWriteExt, SinkExt, StreamExt,
    };
    use hala_io::ReadBuf;
    use hala_quic::{QuicConn, QuicStream};

    use crate::handshake::{HandshakeContext, Protocol};

    use super::*;

    pub(super) async fn run_loop(
        id: String,
        listener: QuicListener,
        join_sender: std::sync::mpsc::Sender<()>,
        transport_manager: TransportManager,
    ) {
        while let Some(conn) = listener.accept().await {
            future_spawn(run_conn_loop(id.clone(), conn, transport_manager.clone()));
        }

        match join_sender.send(()) {
            Err(err) => {
                log::trace!("{}, stop accept loop with error, err={}", id, err);
            }
            _ => {
                log::trace!("{}, stop accept loop", id);
            }
        }
    }

    async fn run_conn_loop(id: String, conn: QuicConn, transport_manager: TransportManager) {
        log::info!("{} handle new incoming connection, {:?}", id, conn);

        while let Some(stream) = conn.accept_stream().await {
            future_spawn(run_stream_loop(
                id.clone(),
                stream,
                transport_manager.clone(),
            ));
        }

        log::info!("{} stop stream accept loop, {:?}", id, conn);
    }

    async fn run_stream_loop(id: String, stream: QuicStream, transport_manager: TransportManager) {
        log::info!("{} handle new incoming stream, {:?}", id, stream);

        let (forward_sender, forward_receiver) =
            futures::channel::mpsc::channel(transport_manager.cache_queue_len());
        let (backward_sender, backward_receiver) =
            futures::channel::mpsc::channel(transport_manager.cache_queue_len());

        let context = HandshakeContext {
            from: format!("{:?}", stream.conn.destination_id()),
            to: format!("{:?}", stream.conn.source_id()),
            protocol: Protocol::Quic,
            forward: forward_receiver,
            backward: backward_sender,
        };

        match transport_manager.handshake(context).await {
            Err(err) => {
                log::error!(
                    "{} handle new incoming stream, {:?}, handshake failed, err={}",
                    id,
                    stream,
                    err
                );

                return;
            }
            _ => {}
        }

        future_spawn(run_stream_forward_loop(
            id.clone(),
            stream.clone(),
            forward_sender,
            transport_manager.max_packet_len(),
        ));

        future_spawn(run_stream_backward_loop(id, stream, backward_receiver));
    }

    async fn run_stream_forward_loop(
        id: String,
        mut stream: QuicStream,
        mut forward_sender: Sender<BytesMut>,
        max_packet_len: usize,
    ) {
        log::info!("{} {:?}, start forward loop", id, stream);

        loop {
            let mut buf = ReadBuf::with_capacity(max_packet_len);

            match stream.stream_recv(buf.as_mut()).await {
                Ok((read_size, fin)) => {
                    match forward_sender
                        .send(buf.into_bytes_mut(Some(read_size)))
                        .await
                    {
                        Err(err) => {
                            log::error!(
                                "{} {:?}, stop forward loop with forward sender error, err={}",
                                id,
                                stream,
                                err
                            );

                            return;
                        }
                        _ => {}
                    }

                    if fin {
                        log::info!("{} {:?}, stop forward loop, client sent fin", id, stream);

                        // try close backward loop
                        _ = stream.close().await;

                        return;
                    }
                }
                Err(err) => {
                    log::error!(
                        "{} {:?}, stop forward loop with stream recv error: err={}",
                        id,
                        stream,
                        err
                    );

                    return;
                }
            }
        }
    }

    async fn run_stream_backward_loop(
        id: String,
        mut stream: QuicStream,
        mut backward_receiver: Receiver<BytesMut>,
    ) {
        log::info!("{} {:?}, start backward loop", id, stream);

        while let Some(buf) = backward_receiver.next().await {
            match stream.write_all(&buf).await {
                Err(err) => {
                    log::error!(
                        "{} {:?}, stop backward loop with stream send error, err={}",
                        id,
                        stream,
                        err
                    );
                }
                _ => {}
            }
        }

        // try close forward loop
        _ = stream.close().await;

        log::error!(
            "{} {:?}, stop backward loop with backward receiver broken.",
            id,
            stream,
        );
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use futures::{
        channel::mpsc::{self, channel},
        future::BoxFuture,
        AsyncReadExt, AsyncWriteExt, SinkExt, StreamExt,
    };
    use hala_io::test::io_test;
    use hala_quic::QuicConn;

    use crate::{
        handshake::{HandshakeResult, Handshaker},
        transport::{Transport, TransportChannel},
    };

    use super::*;

    fn mock_config(is_server: bool, max_datagram_size: usize) -> Config {
        use std::path::Path;

        let mut config = Config::new().unwrap();

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

    struct MockHandshaker {}

    impl Handshaker for MockHandshaker {
        fn handshake(
            &self,
            forward_cx: crate::handshake::HandshakeContext,
        ) -> futures::prelude::future::BoxFuture<
            'static,
            io::Result<crate::handshake::HandshakeResult>,
        > {
            Box::pin(async move {
                Ok(HandshakeResult {
                    context: forward_cx,
                    transport_id: "EchoTransport".into(),
                    conn_str: "".into(),
                })
            })
        }
    }

    struct MockTransport {}

    impl Transport for MockTransport {
        fn id(&self) -> &str {
            "EchoTransport"
        }

        fn open_channel(
            &self,
            _: &str,
            max_packet_len: usize,
            cache_queue_len: usize,
        ) -> BoxFuture<'static, io::Result<crate::transport::TransportChannel>> {
            let (forward_sender, mut forward_receiver) = channel(cache_queue_len);
            let (mut backward_sender, backward_receiver) = channel(cache_queue_len);

            future_spawn(async move {
                while let Some(buf) = forward_receiver.next().await {
                    if backward_sender.send(buf).await.is_err() {
                        return;
                    }
                }
            });

            Box::pin(async move {
                Ok(TransportChannel {
                    max_packet_len,
                    cache_queue_len,
                    sender: forward_sender,
                    receiver: backward_receiver,
                })
            })
        }
    }

    fn mock_tm() -> TransportManager {
        let tm = TransportManager::new(MockHandshaker {}, 1024, 1370);

        tm.register(MockTransport {});

        tm
    }

    #[hala_test::test(io_test)]
    async fn echo_single_thread_test() -> io::Result<()> {
        // pretty_env_logger::init();

        let gateway = QuicGateway::bind("hello", "127.0.0.1:0", mock_config(true, 1350)).unwrap();

        let quic_listener = gateway.listener.clone();

        std::thread::spawn(move || {
            gateway.start(mock_tm()).unwrap();
            gateway.join();
        });

        let raddr = *quic_listener.local_addrs().next().unwrap();

        let conn = QuicConn::connect("127.0.0.1:0", raddr, &mut mock_config(false, 1350))
            .await
            .unwrap();

        let mut stream = conn.open_stream().await.unwrap();

        for i in 0..1000 {
            let data = format!("hello world {}", i);

            stream.write_all(data.as_bytes()).await.unwrap();

            let mut buf = vec![0; 1024];

            let read_size = stream.read(&mut buf).await.unwrap();

            assert_eq!(data.as_bytes(), &buf[..read_size]);
        }

        quic_listener.close().await;

        Ok(())
    }

    #[cold]
    async fn loop_peer_streams_left_bidi(conn: &QuicConn) {
        loop {
            if conn.peer_streams_left_bidi().await > 0 {
                return;
            }
        }
    }

    #[hala_test::test(io_test)]
    async fn echo_close_stream_test() -> io::Result<()> {
        // pretty_env_logger::init();

        let mut config = mock_config(true, 1350);

        config.set_initial_max_streams_bidi(10);

        let gateway = QuicGateway::bind("hello", "127.0.0.1:0", config).unwrap();

        let quic_listener = gateway.listener.clone();

        std::thread::spawn(move || {
            gateway.start(mock_tm()).unwrap();
            gateway.join();
        });

        let raddr = *quic_listener.local_addrs().next().unwrap();

        let conn = QuicConn::connect("127.0.0.1:0", raddr, &mut mock_config(false, 1350))
            .await
            .unwrap();

        for i in 0..1000 {
            {
                let mut stream = conn.open_stream().await.unwrap();

                let data = format!("hello world {}", i);

                stream.write_all(data.as_bytes()).await.unwrap();

                let mut buf = vec![0; 1024];

                let read_size = stream.read(&mut buf).await.unwrap();

                assert_eq!(data.as_bytes(), &buf[..read_size]);
            }

            loop_peer_streams_left_bidi(&conn).await
        }

        quic_listener.close().await;

        Ok(())
    }

    #[hala_test::test(io_test)]
    async fn echo_multi_thread_test() -> io::Result<()> {
        // pretty_env_logger::init();

        let count = 100;

        let mut config = mock_config(true, 1350);

        config.set_initial_max_streams_bidi(count + 1);

        let gateway = QuicGateway::bind("hello", "127.0.0.1:0", config).unwrap();

        let quic_listener = gateway.listener.clone();

        std::thread::spawn(move || {
            gateway.start(mock_tm()).unwrap();
            gateway.join();
        });

        let raddr = *quic_listener.local_addrs().next().unwrap();

        let mut config = mock_config(false, 1350);

        // config.set_initial_max_streams_bidi(count);

        let conn = QuicConn::connect("127.0.0.1:0", raddr, &mut config)
            .await
            .unwrap();

        let (sx, mut rx) = mpsc::channel::<()>(0);

        for i in 0..count {
            let mut stream = conn.open_stream().await.unwrap();

            let mut sx = sx.clone();

            future_spawn(async move {
                for j in 0..count {
                    let data = format!("hello world {}{}", i, j);

                    stream.write_all(data.as_bytes()).await.unwrap();

                    let mut buf = vec![0; 1024];

                    let read_size = stream.read(&mut buf).await.unwrap();

                    assert_eq!(data.as_bytes(), &buf[..read_size]);
                }

                sx.send(()).await.unwrap();
            })
        }

        for _ in 0..count {
            rx.next().await;
        }

        quic_listener.close().await;

        Ok(())
    }
}
