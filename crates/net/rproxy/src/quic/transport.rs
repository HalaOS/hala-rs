use std::{collections::HashMap, io, net::SocketAddr, sync::Arc};

use futures::{channel::mpsc, future::BoxFuture, SinkExt, StreamExt};
use hala_future::executor::future_spawn;
use hala_quic::{Config, QuicConn, QuicConnectionId};
use hala_sync::{AsyncLockable, AsyncSpinMutex};

use crate::transport::{Transport, TransportChannel};

/// The inner state of [`QuicTransport`].
#[derive(Default)]
struct QuicTransportState {
    peer_open_channels: HashMap<String, mpsc::Sender<TransportChannel>>,
}

/// Quic transport protocol.
#[derive(Clone)]
pub struct QuicTransport {
    id: String,
    state: Arc<AsyncSpinMutex<QuicTransportState>>,
    create_config: Arc<Box<dyn Fn(&str) -> Config + Sync + Send + 'static>>,
}

impl QuicTransport {
    /// Create new instance with customer transport id.
    pub fn new<F>(id: String, create_config: F) -> Self
    where
        F: Fn(&str) -> Config + Sync + Send + 'static,
    {
        Self {
            id,
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
        conn_str: &str,
        max_packet_len: usize,
        cache_queue_len: usize,
    ) -> BoxFuture<'static, std::io::Result<TransportChannel>> {
        let conn_str = conn_str.to_owned();

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
                .get_new_channel_sender(&conn_str, max_packet_len)
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
        conn_str: &str,
        max_packet_len: usize,
    ) -> io::Result<mpsc::Sender<TransportChannel>> {
        let mut state_unlocked = self.state.lock().await;

        if let Some(new_channel_sender) = state_unlocked.peer_open_channels.get_mut(conn_str) {
            Ok(new_channel_sender.clone())
        } else {
            let (conn_pool, new_channel_sender) =
                QuicConnPool::new(self.clone(), conn_str.to_string(), 3, max_packet_len)?;

            state_unlocked
                .peer_open_channels
                .insert(conn_str.to_string(), new_channel_sender.clone());

            future_spawn(conn_pool.start());

            Ok(new_channel_sender)
        }
    }
}

struct QuicConnPool {
    conn_str: String,
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
        conn_str: String,
        retry_times: usize,
        max_packet_len: usize,
    ) -> io::Result<(QuicConnPool, mpsc::Sender<TransportChannel>)> {
        let mut raddrs: Vec<SocketAddr> = vec![];

        for raddr in conn_str.split(";") {
            raddrs.push(raddr.parse().map_err(|err| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("parse conn_str failed, split={}, {}", raddr, err),
                )
            })?);
        }

        let (new_channel_sender, new_channel_receiver) = mpsc::channel(1024);

        Ok((
            Self {
                conn_str,
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
                log::info!("quic conn pool stop event loop, conn_str={}", self.conn_str,)
            }
            Err(err) => {
                log::error!(
                    "quic conn pool stop event loop, conn_str={}, err={}",
                    self.conn_str,
                    err
                )
            }
        }
    }

    async fn get_conn(&mut self) -> io::Result<QuicConn> {
        for (_, conn) in self.conns.iter() {
            if conn.peer_streams_left_bidi().await > 0 {
                return Ok(conn.clone());
            }
        }

        let mut config = self.create_config();

        let conn = QuicConn::connect("0.0.0.0:0", self.raddrs.as_slice(), &mut config).await?;

        self.conns.insert(conn.source_id().clone(), conn.clone());

        Ok(conn)
    }

    async fn start_channel_event_loop(&mut self, channel: TransportChannel) {
        // connect retry loop.
        for i in 0..self.retry_times {
            let conn = match self.get_conn().await {
                Ok(conn) => conn,
                Err(err) => {
                    log::error!(
                        "Open quic conn failed, try again({}). conn_str={}, err={}",
                        self.retry_times - i,
                        self.conn_str,
                        err
                    );

                    continue;
                }
            };
            match conn.open_stream().await {
                Ok(stream) => {
                    self.conns.insert(conn.source_id().clone(), conn);
                    future_spawn(event_loops::channel_event_loop(
                        self.conn_str.clone(),
                        stream,
                        channel,
                        self.max_packet_len,
                    ));
                    break;
                }
                Err(err) => {
                    log::error!(
                        "{:?} open stream error,try again({}). conn_str={}, err={}",
                        conn,
                        self.retry_times - i,
                        self.conn_str,
                        err
                    );

                    // remove the connection that returned an error when open_stream was called.
                    self.conns.remove(conn.destination_id());
                }
            }
        }
    }

    fn create_config(&self) -> Config {
        (self.transport.create_config)(&self.conn_str)
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

    use crate::transport::TransportChannel;

    pub(super) async fn channel_event_loop(
        conn_str: String,
        stream: QuicStream,
        channel: TransportChannel,
        max_packet_len: usize,
    ) {
        log::trace!(
            "Open transport channel. conn_str={}, {:?}, channel={:?}",
            conn_str,
            stream,
            channel
        );

        future_spawn(channel_send_event_loop(
            conn_str.clone(),
            channel.uuid.clone(),
            stream.clone(),
            channel.receiver,
        ));

        future_spawn(channel_recv_event_loop(
            conn_str,
            channel.uuid,
            stream,
            channel.sender,
            max_packet_len,
        ));
    }

    async fn channel_send_event_loop(
        conn_str: String,
        channel_id: Uuid,
        mut stream: QuicStream,
        mut forward_receiver: mpsc::Receiver<BytesMut>,
    ) {
        while let Some(buf) = forward_receiver.next().await {
            match stream.write_all(&buf).await {
                Ok(_) => {}
                Err(err) => {
                    log::error!(
                        "stop send loop. conn_str={}, channel={:?}, {:?}, err={}",
                        conn_str,
                        channel_id,
                        stream,
                        err
                    );

                    return;
                }
            }
        }

        _ = stream.close().await;

        log::error!(
            "stop send loop. conn_str={}, channel={:?}, {:?}, err=forward channel closed",
            conn_str,
            channel_id,
            stream,
        );
    }

    async fn channel_recv_event_loop(
        conn_str: String,
        channel_id: Uuid,
        mut stream: QuicStream,
        mut backward_sender: mpsc::Sender<BytesMut>,
        max_packet_len: usize,
    ) {
        loop {
            let mut buf = ReadBuf::with_capacity(max_packet_len);

            match stream.stream_recv(buf.as_mut()).await {
                Ok((read_size, fin)) => {
                    let buf = buf.into_bytes_mut(Some(read_size));

                    if backward_sender.send(buf).await.is_err() {
                        log::error!(
                            "stop recv loop. conn_str={}, channel={:?}, {:?}, err=backward pipe broken",
                            conn_str,
                            channel_id,
                            stream,
                        );

                        _ = stream.close().await;

                        return;
                    }

                    if fin {
                        log::error!(
                            "stop recv loop. conn_str={}, channel={:?}, {:?}, err=peer sent pin",
                            conn_str,
                            channel_id,
                            stream,
                        );

                        _ = stream.close().await;

                        return;
                    }
                }
                Err(err) => {
                    log::error!(
                        "stop recv loop. conn_str={}, channel={:?}, {:?}, err={}",
                        conn_str,
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
