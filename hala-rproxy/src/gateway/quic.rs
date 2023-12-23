use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
    sync::Arc,
};

use bytes::BytesMut;
use futures::{
    channel::mpsc::{Receiver, Sender},
    select, AsyncReadExt, AsyncWriteExt, Future, FutureExt, SinkExt, StreamExt,
};
use hala_io_util::{local_block_on, local_io_spawn, ReadBuf};
use hala_net::quic::{Config, QuicConn, QuicListener, QuicStream};

use crate::forward::RoutingTable;

use super::{GatewayConfig, GatewayController};

pub trait QuicGatewayHandshake {
    type Fut<'a>: Future<Output = io::Result<(Sender<BytesMut>, Receiver<BytesMut>)>>
        + Send
        + Unpin
        + 'a
    where
        Self: 'a;

    /// Invoke async handshake procedure.
    fn handshake<'a>(
        &'a self,
        stream: &'a mut QuicStream,
        routing_table: &'a RoutingTable,
    ) -> Self::Fut<'a>;
}

/// Config for [`TcpGateway`]
pub struct QuicGatewayConfig<H> {
    key: String,
    laddrs: Option<Vec<SocketAddr>>,
    handshake: Option<H>,
    config: Option<Config>,
}

impl<H> QuicGatewayConfig<H> {
    pub fn new<K: Into<String>>(
        key: K,
        laddrs: Vec<SocketAddr>,
        handshake: H,
        config: Config,
    ) -> Self {
        Self {
            key: key.into(),
            laddrs: Some(laddrs),
            config: Some(config),
            handshake: Some(handshake),
        }
    }
}

impl<H> GatewayConfig for QuicGatewayConfig<H>
where
    H: QuicGatewayHandshake + Send + Clone + 'static,
{
    fn key(&self) -> &str {
        self.key.as_str()
    }

    fn start(&mut self, routing_table: RoutingTable) -> std::io::Result<GatewayController> {
        assert!(self.handshake.is_some(), "Call start twice.");

        let (controller, stop_notifier) = GatewayController::new(&self.key);

        let handshake = self.handshake.take().unwrap();

        let config = self.config.take().unwrap();

        let laddrs = self.laddrs.take().unwrap();

        let laddrs = laddrs.clone();

        std::thread::spawn(move || {
            local_block_on(async move {
                let gateway = QuicGateway::new(
                    &laddrs.as_slice(),
                    routing_table,
                    stop_notifier,
                    handshake,
                    config,
                )
                .unwrap();

                gateway.run_loop().await.unwrap();
            })
        });

        Ok(controller)
    }
}

pub struct QuicGateway<H> {
    listener: QuicListener,
    routing_table: RoutingTable,
    stop_notifier: Receiver<()>,
    handshake: H,
}

impl<H> QuicGateway<H> {
    fn new<L: ToSocketAddrs>(
        laddrs: &L,
        routing_table: RoutingTable,
        stop_notifier: Receiver<()>,
        handshake: H,
        config: Config,
    ) -> io::Result<Self> {
        Ok(Self {
            listener: QuicListener::bind(laddrs, config)?,
            routing_table,
            stop_notifier,
            handshake,
        })
    }
}

impl<H> QuicGateway<H>
where
    H: QuicGatewayHandshake + Clone + 'static,
{
    async fn run_loop(mut self) -> io::Result<()> {
        loop {
            select! {
                incoming = self.listener.accept().fuse() => {
                    match incoming {
                        Some(conn) => {

                           let conn_gateway = QuicConnGateway {
                                conn,
                                routing_table: self.routing_table.clone(),
                                handshake: self.handshake.clone(),
                            };

                            local_io_spawn(conn_gateway.run_loop())?;
                        }
                        None => {

                            return Ok(());
                        }
                    }
                }
                _ = self.stop_notifier.next().fuse() => {
                    return Ok(())
                }
            }
        }
    }
}

struct QuicConnGateway<H> {
    conn: QuicConn,
    routing_table: RoutingTable,
    handshake: H,
}

impl<H> QuicConnGateway<H>
where
    H: QuicGatewayHandshake + 'static,
{
    async fn run_loop(self) -> io::Result<()> {
        while let Some(mut stream) = self.conn.accept().await {
            let (sender, receiver) = self
                .handshake
                .handshake(&mut stream, &self.routing_table)
                .await?;

            let stream = Arc::new(stream);

            let forward = QuicGatewaySendTunnel {
                stream: stream.clone(),
                sender,
            };

            let backword = QuicGatewayRecvTunnel {
                stream: stream.clone(),
                receiver,
            };

            // Start tunnel event loop
            local_io_spawn(forward.run_loop())?;

            local_io_spawn(backword.run_loop())?;
        }

        Ok(())
    }
}

struct QuicGatewaySendTunnel {
    stream: Arc<QuicStream>,
    sender: Sender<BytesMut>,
}

impl QuicGatewaySendTunnel {
    async fn run_loop(mut self) -> io::Result<()> {
        loop {
            let mut buf = ReadBuf::with_capacity(65535);

            let read_size = (&*self.stream).read(buf.as_mut()).await?;

            let bytes = buf.into_bytes_mut(Some(read_size));

            match self.sender.send(bytes).await {
                Err(err) => {
                    // close stream.
                    (&*self.stream).close().await?;

                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        format!(
                            "broken gateway send tunnel: trace_id={}, err={}",
                            self.stream.trace_id(),
                            err
                        ),
                    ));
                }
                _ => {}
            };
        }
    }
}

struct QuicGatewayRecvTunnel {
    stream: Arc<QuicStream>,
    receiver: Receiver<BytesMut>,
}

impl QuicGatewayRecvTunnel {
    async fn run_loop(mut self) -> io::Result<()> {
        loop {
            match self.receiver.next().await {
                None => {
                    (&*self.stream).close().await?;

                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        format!("broken gateway recv tunnel: {}", self.stream.trace_id()),
                    ));
                }
                Some(buf) => (&*self.stream).write_all(&buf).await?,
            };
        }
    }
}
