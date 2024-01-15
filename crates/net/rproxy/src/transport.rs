use std::{io, sync::Arc};

use bytes::BytesMut;
use dashmap::DashMap;
use futures::{
    channel::mpsc::{Receiver, Sender},
    future::BoxFuture,
};
use hala_future::executor::future_spawn;

use crate::handshake::{HandshakeContext, Handshaker};

/// Transport channel type create by [`open_channel`](Transport::open_channel) function
pub struct TransportChannel {
    pub max_packet_len: usize,
    pub cache_queue_len: usize,
    pub sender: Sender<BytesMut>,
    pub receiver: Receiver<BytesMut>,
}

/// [`Channel`](TransportChannel) factory responsible for forward/backward data forwarding
pub trait Transport {
    /// Open new [`channel`](TransportChannel) instance with `conn_str`.
    fn open_channel(
        &self,
        conn_str: &str,
        max_packet_len: usize,
        cache_queue_len: usize,
    ) -> BoxFuture<'static, io::Result<TransportChannel>>;

    /// Get identity `str` of this transport.
    fn id(&self) -> &str;
}

/// The manager of [`Transport`] instance.
///
#[derive(Clone)]
pub struct TransportManager {
    max_packet_len: usize,
    cache_queue_len: usize,
    handshaker: Arc<Box<dyn Handshaker + Sync + Send + 'static>>,
    transports: Arc<DashMap<String, Box<dyn Transport + Sync + Send + 'static>>>,
}

impl TransportManager {
    /// Create new instance of this type using the [`handshaker`](Handshaker) instance..
    pub fn new<H: Handshaker + Sync + Send + 'static>(
        handshaker: H,
        cache_queue_len: usize,
        max_packet_len: usize,
    ) -> Self {
        Self {
            handshaker: Arc::new(Box::new(handshaker)),
            transports: Default::default(),
            cache_queue_len,
            max_packet_len,
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
    pub async fn handshake(&self, forward_cx: HandshakeContext) -> io::Result<()> {
        let result = self.handshaker.handshake(forward_cx).await?;

        let transport = self
            .transports
            .get(&result.transport_id)
            .ok_or(io::Error::new(
                io::ErrorKind::NotFound,
                format!("transport {} not found.", result.transport_id),
            ))?;

        let channel = transport
            .open_channel(&result.conn_str, self.max_packet_len, self.cache_queue_len)
            .await?;

        event_loop::run_event_loop(result.context, channel);

        Ok(())
    }

    /// Get forwarding packet queue len.
    pub fn cache_queue_len(&self) -> usize {
        self.cache_queue_len
    }

    /// Get max length of packet in forwarding queue.
    pub fn max_packet_len(&self) -> usize {
        self.max_packet_len
    }
}

mod event_loop {
    use futures::{SinkExt, StreamExt};

    use crate::handshake::Protocol;

    use super::*;

    pub(super) fn run_event_loop(forward_cx: HandshakeContext, channel: TransportChannel) {
        let forward_fut = run_forward_loop(
            forward_cx.from.clone(),
            forward_cx.to.clone(),
            forward_cx.protocol.clone(),
            forward_cx.forward,
            channel.sender,
        );

        let backward_fut = run_backward_loop(
            forward_cx.from,
            forward_cx.to,
            forward_cx.protocol,
            channel.receiver,
            forward_cx.backward,
        );

        future_spawn(forward_fut);

        future_spawn(backward_fut);
    }

    async fn run_forward_loop(
        from: String,
        to: String,
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
        from: String,
        to: String,
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
