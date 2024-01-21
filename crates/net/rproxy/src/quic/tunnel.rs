use std::{collections::HashMap, io, net::SocketAddr, sync::Arc};

use async_trait::async_trait;
use futures::{channel::mpsc, SinkExt, StreamExt};
use hala_future::executor::future_spawn;
use hala_quic::{Config, QuicConn, QuicConnectionId};
use hala_sync::{AsyncLockable, AsyncSpinMutex};

use crate::{
    handshake::TunnelOpenConfiguration,
    transport::TransportConfig,
    tunnel::{Tunnel, TunnelFactory},
};

/// The inner state of [`QuicTransport`].
#[derive(Default)]
struct QuicTransportState {
    peer_open_channels: HashMap<TransportConfig, mpsc::Sender<Tunnel>>,
}

/// Quic transport protocol.
#[derive(Clone)]
pub struct QuicTransport {
    id: String,
    state: Arc<AsyncSpinMutex<QuicTransportState>>,
    create_config: Arc<Box<dyn Fn(&TransportConfig) -> Config + Sync + Send + 'static>>,
}

impl QuicTransport {
    /// Create new instance with customer transport id.
    pub fn new<ID: ToString, F>(id: ID, create_config: F) -> Self
    where
        F: Fn(&TransportConfig) -> Config + Sync + Send + 'static,
    {
        Self {
            id: id.to_string(),
            state: Default::default(),
            create_config: Arc::new(Box::new(create_config)),
        }
    }
}

#[async_trait]
impl TunnelFactory for QuicTransport {
    fn id(&self) -> &str {
        &self.id
    }

    async fn open_tunnel(&self, config: TunnelOpenConfiguration) -> io::Result<Tunnel> {
        let (forward_sender, forward_receiver) = mpsc::channel(config.max_cache_len);

        let (backward_sender, backward_receiver) = mpsc::channel(config.max_cache_len);

        let lhs_channel = Tunnel::new(config.max_packet_len, forward_sender, backward_receiver);

        let rhs_channel = Tunnel::new(config.max_packet_len, backward_sender, forward_receiver);

        let mut sender = self.get_new_channel_sender(config).await?;

        sender
            .send(rhs_channel)
            .await
            .expect("QuicConnPool unexpected stops");

        Ok(lhs_channel)
    }
}

impl QuicTransport {
    async fn get_new_channel_sender(
        &self,
        config: TunnelOpenConfiguration,
    ) -> io::Result<mpsc::Sender<Tunnel>> {
        let mut state_unlocked = self.state.lock().await;

        if let Some(new_channel_sender) = state_unlocked
            .peer_open_channels
            .get_mut(&config.transport_config)
        {
            Ok(new_channel_sender.clone())
        } else {
            let (conn_pool, new_channel_sender) =
                QuicConnPool::new(self.clone(), config.clone(), 3, config.max_packet_len)?;

            state_unlocked
                .peer_open_channels
                .insert(config.transport_config, new_channel_sender.clone());

            future_spawn(conn_pool.start());

            Ok(new_channel_sender)
        }
    }
}

struct QuicConnPool {
    config: TunnelOpenConfiguration,
    laddrs: Vec<SocketAddr>,
    raddrs: Vec<SocketAddr>,
    transport: QuicTransport,
    new_channel_receiver: mpsc::Receiver<Tunnel>,
    conns: HashMap<QuicConnectionId<'static>, QuicConn>,
    retry_times: usize,
    max_packet_len: usize,
}

impl QuicConnPool {
    fn new(
        transport: QuicTransport,
        config: TunnelOpenConfiguration,
        retry_times: usize,
        max_packet_len: usize,
    ) -> io::Result<(QuicConnPool, mpsc::Sender<Tunnel>)> {
        let (laddrs, raddrs) =
            if let TransportConfig::Quic(laddrs, raddrs) = &config.transport_config {
                (laddrs.clone(), raddrs.clone())
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "QuicTransport only accept ChannelOpenFlag::RemoteAddresses",
                ));
            };

        let (new_channel_sender, new_channel_receiver) = mpsc::channel(1024);

        Ok((
            Self {
                config,
                transport,
                new_channel_receiver,
                conns: Default::default(),
                laddrs,
                raddrs,
                retry_times,
                max_packet_len,
            },
            new_channel_sender,
        ))
    }
}

impl QuicConnPool {
    async fn start(mut self) {
        match self.event_loop().await {
            Ok(_) => {
                log::info!(
                    "quic conn pool stop event loop, conn_str={:?}",
                    self.config.transport_config,
                )
            }
            Err(err) => {
                log::trace!(
                    "quic conn pool stop event loop, conn_str={:?}, err={}",
                    self.config.transport_config,
                    err
                )
            }
        }
    }

    async fn get_conn(&mut self) -> io::Result<QuicConn> {
        for (_, conn) in self.conns.iter() {
            // avoid crypto handshake stream.
            if conn.peer_streams_left_bidi().await > 1 {
                return Ok(conn.clone());
            }
        }

        let mut config = self.create_config();

        let conn =
            QuicConn::connect(self.laddrs.as_slice(), self.raddrs.as_slice(), &mut config).await?;

        self.conns.insert(conn.source_id().clone(), conn.clone());

        log::trace!("Create new conn: {:?}", conn);

        Ok(conn)
    }

    async fn start_channel_event_loop(&mut self, channel: Tunnel) {
        let mut last_error = None;

        // connect retry loop.
        for i in 0..self.retry_times {
            let conn = match self.get_conn().await {
                Ok(conn) => conn,
                Err(err) => {
                    log::trace!(
                        "Open quic conn failed, try again({}). open_flag={:?}, err={}",
                        self.retry_times - i,
                        self.config.transport_config,
                        err
                    );

                    last_error = Some(err);

                    continue;
                }
            };
            match conn.open_stream().await {
                Ok(stream) => {
                    self.conns.insert(conn.source_id().clone(), conn);
                    future_spawn(event_loops::channel_event_loop(
                        self.config.clone(),
                        stream,
                        channel,
                        self.max_packet_len,
                    ));
                    return;
                }
                Err(err) => {
                    log::trace!(
                        "{:?} open stream error,try again({}). open_flag={:?}, err={}",
                        conn,
                        self.retry_times - i,
                        self.config.transport_config,
                        err
                    );

                    last_error = Some(err);

                    // remove the connection that returned an error when open_stream was called.
                    self.conns.remove(conn.destination_id());
                }
            }
        }

        log::trace!(
            "open stream error. open_flag={:?}, err={}",
            self.config.transport_config,
            last_error.unwrap(),
        );
    }

    fn create_config(&self) -> Config {
        (self.transport.create_config)(&self.config.transport_config)
    }

    async fn event_loop(&mut self) -> io::Result<()> {
        while let Some(channel) = self.new_channel_receiver.next().await {
            self.start_channel_event_loop(channel).await;
        }

        Ok(())
    }
}

mod event_loops {

    use bytes::BytesMut;
    use futures::{channel::mpsc, AsyncWriteExt, SinkExt, StreamExt};
    use hala_future::executor::future_spawn;
    use hala_io::ReadBuf;
    use hala_quic::QuicStream;
    use uuid::Uuid;

    use crate::{handshake::TunnelOpenConfiguration, tunnel::Tunnel};

    pub(super) async fn channel_event_loop(
        channel_open_flag: TunnelOpenConfiguration,
        stream: QuicStream,
        channel: Tunnel,
        max_packet_len: usize,
    ) {
        log::trace!(
            "Open transport channel. open_flag={:?}, {:?}, channel={:?}",
            channel_open_flag,
            stream,
            channel
        );

        future_spawn(channel_send_event_loop(
            channel_open_flag.clone(),
            channel.uuid.clone(),
            stream.clone(),
            channel.receiver,
        ));

        future_spawn(channel_recv_event_loop(
            channel_open_flag,
            channel.uuid,
            stream,
            channel.sender,
            max_packet_len,
        ));
    }

    async fn channel_send_event_loop(
        channel_open_flag: TunnelOpenConfiguration,
        channel_id: Uuid,
        mut stream: QuicStream,
        mut forward_receiver: mpsc::Receiver<BytesMut>,
    ) {
        log::trace!(
            "start send event loop. open_flag={:?}, channel={:?}, {:?}",
            channel_open_flag,
            channel_id,
            stream,
        );

        while let Some(buf) = forward_receiver.next().await {
            log::trace!("{} forward data: {}", channel_id, buf.len());
            match stream.write_all(&buf).await {
                Ok(_) => {
                    log::trace!("{} forward data: {}, success", channel_id, buf.len());
                }
                Err(err) => {
                    log::trace!(
                        "stop send loop. open_flag={:?}, channel={:?}, {:?}, err={}",
                        channel_open_flag,
                        channel_id,
                        stream,
                        err
                    );

                    _ = stream.close().await;

                    return;
                }
            }
        }

        _ = stream.close().await;

        log::trace!(
            "stop send loop. open_flag={:?}, channel={:?}, {:?}, err=forward channel closed",
            channel_open_flag,
            channel_id,
            stream,
        );
    }

    async fn channel_recv_event_loop(
        channel_open_flag: TunnelOpenConfiguration,
        channel_id: Uuid,
        mut stream: QuicStream,
        mut backward_sender: mpsc::Sender<BytesMut>,
        max_packet_len: usize,
    ) {
        log::trace!(
            "start recv loop. open_flag={:?}, channel={:?}, {:?}",
            channel_open_flag,
            channel_id,
            stream,
        );

        loop {
            let mut buf = ReadBuf::with_capacity(max_packet_len);

            match stream.stream_recv(buf.as_mut()).await {
                Ok((read_size, fin)) => {
                    let buf = buf.into_bytes_mut(Some(read_size));

                    if backward_sender.send(buf).await.is_err() {
                        log::trace!(
                            "stop recv loop. open_flag={:?}, channel={:?}, {:?}, err=backward pipe broken",
                            channel_open_flag,
                            channel_id,
                            stream,
                        );

                        _ = stream.close().await;

                        return;
                    }

                    if fin {
                        log::trace!(
                            "stop recv loop. open_flag={:?}, channel={:?}, {:?}, err=peer sent pin",
                            channel_open_flag,
                            channel_id,
                            stream,
                        );

                        _ = stream.close().await;

                        return;
                    }
                }
                Err(err) => {
                    log::trace!(
                        "stop recv loop. open_flag={:?}, channel={:?}, {:?}, err={}",
                        channel_open_flag,
                        channel_id,
                        stream,
                        err
                    );

                    // try stop send loop
                    _ = stream.close().await;

                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {

    use crate::{
        handshake::{HandshakeContext, Handshaker},
        transport::PathInfo,
        tunnel::TunnelFactoryManager,
    };

    use super::*;

    use std::io;

    use bytes::BytesMut;
    use futures::AsyncWriteExt;
    use hala_future::executor::future_spawn;
    use hala_io::test::io_test;
    use hala_quic::{Config, QuicConn, QuicListener, QuicStream};

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

        config.set_max_idle_timeout(1000);
        config.set_max_recv_udp_payload_size(max_datagram_size);
        config.set_max_send_udp_payload_size(max_datagram_size);
        config.set_initial_max_data(10_000_000);
        config.set_initial_max_stream_data_bidi_local(10_000_000);
        config.set_initial_max_stream_data_bidi_remote(10_000_000);
        config.set_initial_max_streams_bidi(9);
        config.set_initial_max_streams_uni(9);
        config.set_disable_active_migration(false);

        config
    }

    async fn create_echo_server(max_streams: u64) -> QuicListener {
        let mut config = mock_config(true, 1370);

        config.set_initial_max_streams_bidi(max_streams);

        let listener: QuicListener = QuicListener::bind("127.0.0.1:0", config).unwrap();

        let listener_cloned = listener.clone();

        future_spawn(async move {
            while let Some(conn) = listener_cloned.accept().await {
                future_spawn(handle_echo_conn(conn));
            }
        });

        listener
    }

    async fn handle_echo_conn(conn: QuicConn) {
        while let Some(stream) = conn.accept_stream().await {
            future_spawn(handle_echo_stream(stream));
        }
    }

    async fn handle_echo_stream(mut stream: QuicStream) {
        let mut buf = vec![0; 1370];

        loop {
            log::trace!("{:?} begin read", stream);
            let (read_size, fin) = stream.stream_recv(&mut buf).await.unwrap();
            log::trace!("{:?} end read", stream);

            log::trace!("{:?} begin write", stream);
            stream.write_all(&buf[..read_size]).await.unwrap();
            log::trace!("{:?} end write", stream);

            if fin {
                return;
            }
        }
    }

    fn mock_create_config(_flag: &TransportConfig, max_packet_len: usize) -> Config {
        mock_config(false, max_packet_len)
    }

    struct MockHandshaker {
        raddr: SocketAddr,
    }

    #[async_trait]
    impl Handshaker for MockHandshaker {
        async fn handshake(
            &self,
            cx: HandshakeContext,
        ) -> io::Result<(HandshakeContext, TunnelOpenConfiguration)> {
            let raddr = self.raddr.clone();
            let max_packet_len = cx.max_packet_len;
            let max_cache_len = cx.max_cache_len;

            Ok((
                cx,
                TunnelOpenConfiguration {
                    max_packet_len,
                    max_cache_len,
                    tunnel_service_id: "QuicTransport".into(),
                    transport_config: TransportConfig::Quic(
                        vec!["127.0.0.1:0".parse().unwrap()],
                        vec![raddr],
                    ),
                },
            ))
        }
    }

    fn mock_tm(raddr: SocketAddr, max_packet_len: usize) -> TunnelFactoryManager {
        let transport_manager = TunnelFactoryManager::new(MockHandshaker { raddr });

        transport_manager.register(QuicTransport::new("QuicTransport", move |flag| {
            mock_create_config(flag, max_packet_len)
        }));

        transport_manager
    }

    async fn mock_client(
        tm: &TunnelFactoryManager,
        cache_queue_len: usize,
    ) -> (mpsc::Sender<BytesMut>, mpsc::Receiver<BytesMut>) {
        let (sender, forward_receiver) = mpsc::channel(cache_queue_len);
        let (backward_sender, receiver) = mpsc::channel(cache_queue_len);

        let cx = HandshakeContext {
            path: PathInfo::None,
            max_cache_len: 1024,
            max_packet_len: 1370,
            forward: forward_receiver,
            backward: backward_sender,
        };

        tm.handshake(cx).await.unwrap();

        (sender, receiver)
    }

    #[hala_test::test(io_test)]
    async fn echo_single_client() -> io::Result<()> {
        // pretty_env_logger::init_timed();

        let cache_queue_len = 1024;
        let max_packet_len = 1370;

        let listener = create_echo_server(10).await;

        let raddr = *listener.local_addrs().next().unwrap();

        let tm = mock_tm(raddr, max_packet_len);

        let (mut sender, mut receiver) = mock_client(&tm, cache_queue_len).await;

        for i in 0..1000 {
            let send_data = format!("Hello world, {}", i);

            log::info!("begin send {}", i);

            sender
                .send(BytesMut::from(send_data.as_bytes()))
                .await
                .unwrap();

            let buf = receiver.next().await.unwrap();

            assert_eq!(&buf, send_data.as_bytes());
        }

        listener.close().await;

        Ok(())
    }

    #[hala_test::test(io_test)]
    async fn echo_mult_client() -> io::Result<()> {
        let cache_queue_len = 1024;
        let max_packet_len = 1350;

        let listener = create_echo_server(3).await;

        let raddr = *listener.local_addrs().next().unwrap();

        let tm = mock_tm(raddr, max_packet_len);

        for i in 0..100 {
            let (mut sender, mut receiver) = mock_client(&tm, cache_queue_len).await;

            for j in 0..100 {
                let send_data = format!("Hello world, {} {}", i, j);

                sender
                    .send(BytesMut::from(send_data.as_bytes()))
                    .await
                    .unwrap();

                let buf = receiver.next().await.unwrap();

                assert_eq!(&buf, send_data.as_bytes());

                log::info!("{} {}", i, j);
            }

            log::info!("finish {}", i);
        }

        listener.close().await;

        Ok(())
    }

    #[hala_test::test(io_test)]
    async fn echo_mult_client_multi_thread() -> io::Result<()> {
        let cache_queue_len = 1024;
        let max_packet_len = 1350;

        let listener = create_echo_server(3).await;

        let raddr = *listener.local_addrs().next().unwrap();

        let tm = mock_tm(raddr, max_packet_len);

        let (sx, mut rx) = mpsc::channel(0);

        let clients = 100;

        for i in 0..clients {
            let (mut sender, mut receiver) = mock_client(&tm, cache_queue_len).await;

            let mut sx = sx.clone();

            future_spawn(async move {
                log::trace!("start {}", i);

                for j in 0..100 {
                    let send_data = format!("Hello world, {} {}", i, j);

                    sender
                        .send(BytesMut::from(send_data.as_bytes()))
                        .await
                        .unwrap();

                    let buf = receiver.next().await.unwrap();

                    assert_eq!(&buf, send_data.as_bytes());
                }

                sx.send(()).await.unwrap();

                log::trace!("finished {}", i);
            })
        }

        for _ in 0..clients {
            rx.next().await.unwrap();
        }

        listener.close().await;

        Ok(())
    }
}
