use std::{fmt::Debug, io, sync::Arc};

use async_trait::async_trait;
use bytes::BytesMut;
use dashmap::DashMap;
use futures::channel::mpsc::{Receiver, Sender};
use uuid::Uuid;

use crate::handshaker::{BoxHandshaker, HandshakeContext, Handshaker, TunnelOpenConfiguration};

/// Transport channel type create by [`open_channel`](Transport::open_channel) function
pub struct Tunnel {
    pub uuid: Uuid,
    pub max_packet_len: usize,
    pub sender: Sender<BytesMut>,
    pub receiver: Receiver<BytesMut>,
}

impl Tunnel {
    /// Create new channel with random uuid.
    pub fn new(
        max_packet_len: usize,
        sender: Sender<BytesMut>,
        receiver: Receiver<BytesMut>,
    ) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            max_packet_len,
            sender,
            receiver,
        }
    }
}

impl Debug for Tunnel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Tunnel({:?})", self.uuid)
    }
}

#[async_trait]
pub trait TunnelFactory {
    /// Using [`config`](TunnelOpenConfiguration) to open new tunnel instance.
    async fn open_tunnel(&self, config: TunnelOpenConfiguration) -> io::Result<Tunnel>;

    /// Get tunnel service id.
    fn id(&self) -> &str;
}

pub type BoxTunnelService = Box<dyn TunnelFactory + Sync + Send + 'static>;

/// The manager of [`Transport`] instance.
///
#[derive(Clone)]
pub struct TunnelFactoryManager {
    handshaker: Arc<BoxHandshaker>,
    transports: Arc<DashMap<String, BoxTunnelService>>,
}

impl TunnelFactoryManager {
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
    pub fn register<T: TunnelFactory + Send + Sync + 'static>(&self, transport: T) {
        let transport = Box::new(transport);

        let key = transport.id().to_owned();

        assert!(
            self.transports.insert(key.clone(), transport).is_none(),
            "Register {key} twice"
        );
    }

    /// Start a new handshake processing.
    pub async fn handshake(&self, forward_cx: HandshakeContext) -> io::Result<()> {
        let (context, config) = self.handshaker.handshake(forward_cx).await?;

        let tunnel_service =
            self.transports
                .get(&config.tunnel_service_id)
                .ok_or(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("transport {} not found.", config.tunnel_service_id),
                ))?;

        let channel = tunnel_service.open_tunnel(config).await?;

        event_loop::run_event_loop(context, channel);

        Ok(())
    }
}

mod event_loop {
    use futures::{SinkExt, StreamExt};
    use hala_future::executor::future_spawn;

    use crate::{handshaker::HandshakeContext, protocol::PathInfo};

    use super::*;

    pub(super) fn run_event_loop(forward_cx: HandshakeContext, tunnel: Tunnel) {
        let forward_fut =
            run_forward_loop(forward_cx.path.clone(), forward_cx.forward, tunnel.sender);

        let backward_fut = run_backward_loop(forward_cx.path, tunnel.receiver, forward_cx.backward);

        future_spawn(forward_fut);

        future_spawn(backward_fut);
    }

    async fn run_forward_loop(
        path_info: PathInfo,
        mut receiver: Receiver<BytesMut>,
        mut sender: Sender<BytesMut>,
    ) {
        while let Some(data) = receiver.next().await {
            if sender.send(data).await.is_err() {
                log::trace!(
                    "{:?}, stop forward channel, transport sender broken.",
                    path_info
                );

                return;
            }
        }

        log::trace!(
            "{:?}, stop forward channel, gateway receiver broken.",
            path_info
        );
    }

    async fn run_backward_loop(
        path_info: PathInfo,
        mut receiver: Receiver<BytesMut>,
        mut sender: Sender<BytesMut>,
    ) {
        while let Some(data) = receiver.next().await {
            if sender.send(data).await.is_err() {
                log::trace!(
                    "{:?}, stop backward channel, gateway sender broken.",
                    path_info
                );

                return;
            }
        }

        log::trace!(
            "{:?}, stop backward channel, transport receiver broken.",
            path_info
        );
    }
}
