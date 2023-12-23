use std::{
    io,
    net::{Shutdown, SocketAddr},
    sync::Arc,
};

use bytes::BytesMut;
use futures::{
    channel::mpsc::{Receiver, Sender},
    select, AsyncReadExt, AsyncWriteExt, Future, FutureExt, SinkExt, StreamExt,
};
use hala_io_util::{io_spawn, ReadBuf};
use hala_net::{TcpListener, TcpStream};

use crate::forward::RoutingTable;

use super::{GatewayConfig, GatewayController};

/// The gateway handshake protocol must implement this trait.
pub trait TcpGatewayHandshake {
    type Fut<'a>: Future<Output = io::Result<(Sender<BytesMut>, Receiver<BytesMut>)>>
        + Send
        + Unpin
        + 'a
    where
        Self: 'a;

    /// Invoke async handshake procedure.
    fn handshake<'a>(
        &'a self,
        stream: &'a mut TcpStream,
        raddr: SocketAddr,
        routing_table: &'a RoutingTable,
    ) -> Self::Fut<'a>;
}

/// Config for [`TcpGateway`]
pub struct TcpGatewayConfig<H> {
    key: String,
    laddrs: Vec<SocketAddr>,
    handshake: Option<H>,
}

impl<H> TcpGatewayConfig<H> {
    pub fn new<K: Into<String>>(key: K, laddrs: Vec<SocketAddr>, handshake: H) -> Self {
        Self {
            key: key.into(),
            laddrs,
            handshake: Some(handshake),
        }
    }
}

impl<H> GatewayConfig for TcpGatewayConfig<H>
where
    H: TcpGatewayHandshake + Send + Sync + 'static,
{
    fn key(&self) -> &str {
        self.key.as_str()
    }

    fn start(&mut self, routing_table: RoutingTable) -> std::io::Result<GatewayController> {
        assert!(self.handshake.is_some(), "Call start twice.");

        let (controller, stop_notifier) = GatewayController::new(&self.key);

        let gateway = TcpGateway::new(
            &self.laddrs,
            routing_table,
            stop_notifier,
            self.handshake.take().unwrap(),
        )?;

        io_spawn(gateway.run_loop())?;

        Ok(controller)
    }
}

pub struct TcpGateway<H> {
    listener: TcpListener,
    routing_table: RoutingTable,
    stop_notifier: Receiver<()>,
    handshake: H,
}

impl<H> TcpGateway<H> {
    fn new(
        laddrs: &[SocketAddr],
        routing_table: RoutingTable,
        stop_notifier: Receiver<()>,
        handshake: H,
    ) -> io::Result<Self> {
        Ok(TcpGateway {
            listener: TcpListener::bind(laddrs)?,
            routing_table,
            stop_notifier,
            handshake,
        })
    }
}

impl<H> TcpGateway<H>
where
    H: TcpGatewayHandshake + Sync,
{
    async fn run_loop(mut self) -> io::Result<()> {
        loop {
            select! {
                incoming = self.listener.accept().fuse() => {
                    match incoming {
                        Ok((incoming,raddr)) => {
                            match self.handle_incoming(incoming,raddr).await {
                                Err(err) => {
                                    log::error!("handle incoming conn err, err={}",err);
                                }
                                _ => {}
                            }
                        }
                        Err(err) => {
                            return Err(err);
                        }
                    }
                }
                _ = self.stop_notifier.next().fuse() => {
                    return Ok(())
                }
            }
        }
    }

    async fn handle_incoming(&self, mut stream: TcpStream, raddr: SocketAddr) -> io::Result<()> {
        log::trace!("tcp gateway incoming, raddr={}", raddr);

        let (sender, receiver) = self
            .handshake
            .handshake(&mut stream, raddr.clone(), &self.routing_table)
            .await?;

        log::trace!("tcp gateway incoming, raddr={}, handshake succeed", raddr);

        let stream = Arc::new(stream);

        let forward = TcpGatewaySendTunnel {
            stream: stream.clone(),
            sender,
            raddr: raddr.clone(),
        };

        let backword = TcpGatewayRecvTunnel {
            stream: stream.clone(),
            receiver,
            raddr,
        };

        // Start tunnel event loop
        io_spawn(forward.run_loop())?;

        io_spawn(backword.run_loop())
    }
}

struct TcpGatewaySendTunnel {
    stream: Arc<TcpStream>,
    sender: Sender<BytesMut>,
    raddr: SocketAddr,
}

impl TcpGatewaySendTunnel {
    async fn run_loop(mut self) -> io::Result<()> {
        loop {
            let mut buf = ReadBuf::with_capacity(65535);

            let read_size = (&*self.stream).read(buf.as_mut()).await?;

            // tcp stream read shutdown
            if read_size == 0 {
                return Ok(());
            }

            let bytes = buf.into_bytes_mut(Some(read_size));

            match self.sender.send(bytes).await {
                Err(err) => {
                    self.stream.shutdown(Shutdown::Both)?;

                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        format!(
                            "broken gateway send tunnel: raddr={}, err={}",
                            self.raddr, err
                        ),
                    ));
                }
                _ => {}
            };
        }
    }
}

struct TcpGatewayRecvTunnel {
    stream: Arc<TcpStream>,
    receiver: Receiver<BytesMut>,
    raddr: SocketAddr,
}

impl TcpGatewayRecvTunnel {
    async fn run_loop(mut self) -> io::Result<()> {
        loop {
            match self.receiver.next().await {
                None => {
                    self.stream.shutdown(Shutdown::Both)?;
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        format!("broken gateway recv tunnel: raddr={}", self.raddr),
                    ));
                }
                Some(buf) => (&*self.stream).write_all(&buf).await?,
            };
        }
    }
}
