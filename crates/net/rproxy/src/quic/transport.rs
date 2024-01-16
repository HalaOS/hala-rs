use std::{collections::HashMap, io, net::SocketAddr, sync::Arc};

use futures::{channel::mpsc, future::BoxFuture, SinkExt, StreamExt};
use hala_future::executor::future_spawn;
use hala_quic::{Config, QuicConn, QuicConnectionId};
use hala_sync::{AsyncLockable, AsyncSpinMutex};

use crate::transport::{ChannelOpenFlag, Transport, TransportChannel};

/// The inner state of [`QuicTransport`].
#[derive(Default)]
struct QuicTransportState {
    peer_open_channels: HashMap<ChannelOpenFlag, mpsc::Sender<TransportChannel>>,
}

/// Quic transport protocol.
#[derive(Clone)]
pub struct QuicTransport {
    id: String,
    state: Arc<AsyncSpinMutex<QuicTransportState>>,
    create_config: Arc<Box<dyn Fn(&ChannelOpenFlag) -> Config + Sync + Send + 'static>>,
}

impl QuicTransport {
    /// Create new instance with customer transport id.
    pub fn new<ID: ToString, F>(id: ID, create_config: F) -> Self
    where
        F: Fn(&ChannelOpenFlag) -> Config + Sync + Send + 'static,
    {
        Self {
            id: id.to_string(),
            state: Default::default(),
            create_config: Arc::new(Box::new(create_config)),
        }
    }
}

impl Transport for QuicTransport {
    fn id(&self) -> &str {
        &self.id
    }

    fn open_channel(
        &self,
        openflag: ChannelOpenFlag,
        max_packet_len: usize,
        cache_queue_len: usize,
    ) -> BoxFuture<'static, std::io::Result<TransportChannel>> {
        let this = self.clone();

        Box::pin(async move {
            let (forward_sender, forward_receiver) = mpsc::channel(cache_queue_len);

            let (backward_sender, backward_receiver) = mpsc::channel(cache_queue_len);

            let lhs_channel = TransportChannel::new(
                max_packet_len,
                cache_queue_len,
                forward_sender,
                backward_receiver,
            );

            let rhs_channel = TransportChannel::new(
                max_packet_len,
                cache_queue_len,
                backward_sender,
                forward_receiver,
            );

            let mut sender = this
                .get_new_channel_sender(openflag, max_packet_len)
                .await?;

            sender
                .send(rhs_channel)
                .await
                .expect("QuicConnPool unexpected stops");

            Ok(lhs_channel)
        })
    }
}

impl QuicTransport {
    async fn get_new_channel_sender(
        &self,
        channel_open_flag: ChannelOpenFlag,
        max_packet_len: usize,
    ) -> io::Result<mpsc::Sender<TransportChannel>> {
        let mut state_unlocked = self.state.lock().await;

        if let Some(new_channel_sender) = state_unlocked
            .peer_open_channels
            .get_mut(&channel_open_flag)
        {
            Ok(new_channel_sender.clone())
        } else {
            let (conn_pool, new_channel_sender) =
                QuicConnPool::new(self.clone(), channel_open_flag.clone(), 3, max_packet_len)?;

            state_unlocked
                .peer_open_channels
                .insert(channel_open_flag, new_channel_sender.clone());

            future_spawn(conn_pool.start());

            Ok(new_channel_sender)
        }
    }
}

struct QuicConnPool {
    channel_open_flag: ChannelOpenFlag,
    raddrs: Vec<SocketAddr>,
    transport: QuicTransport,
    new_channel_receiver: mpsc::Receiver<TransportChannel>,
    conns: HashMap<QuicConnectionId<'static>, QuicConn>,
    retry_times: usize,
    max_packet_len: usize,
}

impl QuicConnPool {
    fn new(
        transport: QuicTransport,
        channel_open_flag: ChannelOpenFlag,
        retry_times: usize,
        max_packet_len: usize,
    ) -> io::Result<(QuicConnPool, mpsc::Sender<TransportChannel>)> {
        let raddrs = if let ChannelOpenFlag::RemoteAddresses(raddrs) = &channel_open_flag {
            raddrs.clone()
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "QuicTransport only accept ChannelOpenFlag::RemoteAddresses",
            ));
        };

        let (new_channel_sender, new_channel_receiver) = mpsc::channel(1024);

        Ok((
            Self {
                channel_open_flag,
                transport,
                new_channel_receiver,
                conns: Default::default(),
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
                    self.channel_open_flag,
                )
            }
            Err(err) => {
                log::trace!(
                    "quic conn pool stop event loop, conn_str={:?}, err={}",
                    self.channel_open_flag,
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

        let conn = QuicConn::connect("0.0.0.0:0", self.raddrs.as_slice(), &mut config).await?;

        self.conns.insert(conn.source_id().clone(), conn.clone());

        log::trace!("Create new conn: {:?}", conn);

        Ok(conn)
    }

    async fn start_channel_event_loop(&mut self, channel: TransportChannel) {
        let mut last_error = None;

        // connect retry loop.
        for i in 0..self.retry_times {
            let conn = match self.get_conn().await {
                Ok(conn) => conn,
                Err(err) => {
                    log::trace!(
                        "Open quic conn failed, try again({}). open_flag={:?}, err={}",
                        self.retry_times - i,
                        self.channel_open_flag,
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
                        self.channel_open_flag.clone(),
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
                        self.channel_open_flag,
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
            self.channel_open_flag,
            last_error.unwrap(),
        );
    }

    fn create_config(&self) -> Config {
        (self.transport.create_config)(&self.channel_open_flag)
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

    use crate::transport::{ChannelOpenFlag, TransportChannel};

    pub(super) async fn channel_event_loop(
        channel_open_flag: ChannelOpenFlag,
        stream: QuicStream,
        channel: TransportChannel,
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
        channel_open_flag: ChannelOpenFlag,
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
            match stream.write_all(&buf).await {
                Ok(_) => {}
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
        channel_open_flag: ChannelOpenFlag,
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

    use super::*;

    use std::io;

    use bytes::BytesMut;
    use futures::AsyncWriteExt;
    use hala_future::executor::future_spawn;
    use hala_io::test::io_test;
    use hala_quic::{Config, QuicConn, QuicListener, QuicStream};

    use crate::{
        handshake::{HandshakeContext, HandshakeResult, Handshaker, Protocol},
        transport::{ChannelOpenFlag, TransportManager},
    };

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

        config.set_max_idle_timeout(10000);
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
            let (read_size, fin) = stream.stream_recv(&mut buf).await.unwrap();

            stream.write_all(&buf[..read_size]).await.unwrap();

            if fin {
                return;
            }
        }
    }

    fn mock_create_config(_flag: &ChannelOpenFlag, max_packet_len: usize) -> Config {
        mock_config(false, max_packet_len)
    }

    struct MockHandshaker {
        raddr: SocketAddr,
    }

    impl Handshaker for MockHandshaker {
        fn handshake(
            &self,
            forward_cx: crate::handshake::HandshakeContext,
        ) -> futures::prelude::future::BoxFuture<
            'static,
            io::Result<crate::handshake::HandshakeResult>,
        > {
            let raddr = self.raddr.clone();

            Box::pin(async move {
                Ok(HandshakeResult {
                    context: forward_cx,
                    transport_id: "QuicTransport".into(),
                    channel_open_flag: ChannelOpenFlag::RemoteAddresses(vec![raddr]),
                })
            })
        }
    }

    fn mock_tm(
        raddr: SocketAddr,
        cache_queue_len: usize,
        max_packet_len: usize,
    ) -> TransportManager {
        let transport_manager =
            TransportManager::new(MockHandshaker { raddr }, cache_queue_len, max_packet_len);

        transport_manager.register(QuicTransport::new("QuicTransport", move |flag| {
            mock_create_config(flag, max_packet_len)
        }));

        transport_manager
    }

    async fn mock_client(
        tm: &TransportManager,
        cache_queue_len: usize,
    ) -> (mpsc::Sender<BytesMut>, mpsc::Receiver<BytesMut>) {
        let (sender, forward_receiver) = mpsc::channel(cache_queue_len);
        let (backward_sender, receiver) = mpsc::channel(cache_queue_len);

        let cx = HandshakeContext {
            from: "".into(),
            to: "".into(),
            protocol: Protocol::Other("test".into()),
            forward: forward_receiver,
            backward: backward_sender,
        };

        tm.handshake(cx).await.unwrap();

        (sender, receiver)
    }

    #[hala_test::test(io_test)]
    async fn echo_single_client() -> io::Result<()> {
        let cache_queue_len = 1024;
        let max_packet_len = 1350;

        let listener = create_echo_server(10).await;

        let raddr = *listener.local_addrs().next().unwrap();

        let tm = mock_tm(raddr, cache_queue_len, max_packet_len);

        let (mut sender, mut receiver) = mock_client(&tm, cache_queue_len).await;

        for i in 0..1000 {
            let send_data = format!("Hello world, {}", i);

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

        let tm = mock_tm(raddr, cache_queue_len, max_packet_len);

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
            }
        }

        listener.close().await;

        Ok(())
    }

    #[hala_test::test(io_test)]
    async fn echo_mult_client_multi_thread() -> io::Result<()> {
        // pretty_env_logger::init();

        let cache_queue_len = 1024;
        let max_packet_len = 1350;

        let listener = create_echo_server(3).await;

        let raddr = *listener.local_addrs().next().unwrap();

        let tm = mock_tm(raddr, cache_queue_len, max_packet_len);

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
