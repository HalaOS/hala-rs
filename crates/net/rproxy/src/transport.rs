use std::{io, net::SocketAddr, sync::Arc};

use bytes::BytesMut;
use dashmap::DashMap;
use futures::{
    channel::mpsc::{Receiver, Sender},
    future::BoxFuture,
};
use hala_future::executor::future_spawn;

/// The transport layer protocol of gateway.
#[derive(Debug, Clone)]
pub enum Protocol {
    Tcp,
    Quic,
    Http1,
    Http2,
    Http3,
    Other(String),
}

/// Forwarding channel creation context.
pub struct ForwardContext {
    /// Connection from endpoint
    pub from: SocketAddr,
    /// Connection to endpoint.
    pub to: SocketAddr,
    /// Gateway transport layer protocol.
    pub protocol: Protocol,
    /// Backward data sender.
    pub backward: Sender<BytesMut>,
    /// Forward data receiver.
    pub forward: Receiver<BytesMut>,
}

/// The result returns by [`handshake`](Handshaker::handshake) function
pub struct HandshakeResult {
    pub context: ForwardContext,
    pub transport_id: String,
    pub conn_str: String,
}

/// Handshake protocol type.
pub trait Handshaker {
    /// Start a new handshake async task.
    ///
    /// If successful, returns [`HandshakeResult`]
    fn handshake(
        &self,
        forward_cx: ForwardContext,
    ) -> BoxFuture<'static, io::Result<HandshakeResult>>;
}

/// Transport channel type create by [`open_channel`](Transport::open_channel) function
pub struct TransportChannel {
    pub forward: Sender<BytesMut>,
    pub backward: Receiver<BytesMut>,
}

/// [`Channel`](TransportChannel) factory responsible for forward/backward data forwarding
pub trait Transport {
    /// Open new [`channel`](TransportChannel) instance with `conn_str`.
    fn open_channel(&self, conn_str: &str) -> io::Result<TransportChannel>;

    /// Get identity `str` of this transport.
    fn id(&self) -> &str;
}

/// The manager of [`Transport`] instance.
///
#[derive(Clone)]
pub struct TransportManager {
    handshaker: Arc<Box<dyn Handshaker + Sync + Send + 'static>>,
    transports: Arc<DashMap<String, Box<dyn Transport + Sync + Send + 'static>>>,
}

impl TransportManager {
    /// Create new instance of this type using the [`handshaker`](Handshaker) instance..
    pub fn new<H: Handshaker + Sync + Send + 'static>(handshaker: H) -> Self {
        Self {
            handshaker: Arc::new(Box::new(handshaker)),
            transports: Default::default(),
        }
    }

    /// Register transport.
    ///
    /// If the same ID is used to register the transport twice, the function will panic.
    pub fn register<T: Transport + Send + Sync + 'static>(&self, transport: T) {
        let transport = Box::new(transport);

        let key = transport.id().to_owned();

        assert!(
            self.transports.insert(key.clone(), transport).is_none(),
            "Register {key} twice"
        );
    }

    /// Start a new handshake processing.
    pub async fn handshake(&self, forward_cx: ForwardContext) -> io::Result<()> {
        let result = self.handshaker.handshake(forward_cx).await?;

        let transport = self
            .transports
            .get(&result.transport_id)
            .ok_or(io::Error::new(
                io::ErrorKind::NotFound,
                format!("transport {} not found.", result.transport_id),
            ))?;

        let channel = transport.open_channel(&result.conn_str)?;

        event_loop::run_event_loop(result.context, channel);

        Ok(())
    }
}

mod event_loop {
    use futures::{SinkExt, StreamExt};

    use super::*;

    pub(super) fn run_event_loop(forward_cx: ForwardContext, channel: TransportChannel) {
        let forward_fut = run_forward_loop(
            forward_cx.from.clone(),
            forward_cx.to.clone(),
            forward_cx.protocol.clone(),
            forward_cx.forward,
            channel.forward,
        );

        let backward_fut = run_backward_loop(
            forward_cx.from,
            forward_cx.to,
            forward_cx.protocol,
            channel.backward,
            forward_cx.backward,
        );

        future_spawn(forward_fut);

        future_spawn(backward_fut);
    }

    async fn run_forward_loop(
        from: SocketAddr,
        to: SocketAddr,
        protocol: Protocol,
        mut receiver: Receiver<BytesMut>,
        mut sender: Sender<BytesMut>,
    ) {
        while let Some(data) = receiver.next().await {
            if sender.send(data).await.is_err() {
                log::trace!(
                    "{:?} from={:?}, to={:?}, stop forward channel, transport sender broken.",
                    protocol,
                    from,
                    to,
                );

                return;
            }
        }

        log::trace!(
            "{:?} from={:?}, to={:?}, stop forward channel, gateway receiver broken.",
            protocol,
            from,
            to,
        );
    }

    async fn run_backward_loop(
        from: SocketAddr,
        to: SocketAddr,
        protocol: Protocol,
        mut receiver: Receiver<BytesMut>,
        mut sender: Sender<BytesMut>,
    ) {
        while let Some(data) = receiver.next().await {
            if sender.send(data).await.is_err() {
                log::trace!(
                    "{:?} from={:?}, to={:?}, stop backward channel, gateway sender broken.",
                    protocol,
                    from,
                    to,
                );

                return;
            }
        }

        log::trace!(
            "{:?} from={:?}, to={:?}, stop backward channel, transport receiver broken.",
            protocol,
            from,
            to,
        );
    }
}
