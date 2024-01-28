use std::{fmt::Debug, io, sync::Arc};

use async_trait::async_trait;
use bytes::BytesMut;
use dashmap::DashMap;
use futures::{
    channel::mpsc::{self, Receiver, Sender},
    SinkExt,
};
use uuid::Uuid;

use crate::{
    handshaker::{BoxHandshaker, HandshakeContext, Handshaker, TunnelOpenConfig},
    profile::Sample,
    TransportConfig,
};

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
    async fn open_tunnel(&self, config: TunnelOpenConfig) -> io::Result<()>;

    /// Get tunnel service id.
    fn id(&self) -> &str;

    /// generate sample data.
    fn sample(&self) -> Sample;
}

type BoxTunnelFactory = Box<dyn TunnelFactory + Sync + Send + 'static>;

/// The manager of [`Transport`] instance.
///
#[derive(Clone)]
pub struct TunnelFactoryManager {
    handshaker: Arc<BoxHandshaker>,
    transports: Arc<DashMap<String, BoxTunnelFactory>>,
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
        let config = self.handshaker.handshake(forward_cx).await?;

        let tunnel_service =
            self.transports
                .get(&config.tunnel_service_id)
                .ok_or(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("transport {} not found.", config.tunnel_service_id),
                ))?;

        tunnel_service.open_tunnel(config).await
    }

    pub fn sample(&self) -> Vec<Sample> {
        let mut samples = vec![];

        for tunnel_factory in self.transports.iter() {
            samples.push(tunnel_factory.sample());
        }

        samples
    }
}

/// The rhs tunnel sender.
pub struct TunnelFactorySender {
    name: String,
    sender: mpsc::Sender<(Tunnel, TransportConfig)>,
}

/// The rhs tunnel receiver.
pub type TunnelFactoryReceiver = mpsc::Receiver<(Tunnel, TransportConfig)>;

#[async_trait]
impl TunnelFactory for TunnelFactorySender {
    /// Using [`config`](TunnelOpenConfiguration) to open new tunnel instance.
    async fn open_tunnel(&self, config: TunnelOpenConfig) -> io::Result<()> {
        let rhs_tunnel = Tunnel::new(
            config.max_packet_len,
            config.gateway_backward,
            config.gateway_forward,
        );

        self.sender
            .clone()
            .send((rhs_tunnel, config.transport_config))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Send tunnel rhs error"))?;

        Ok(())
    }

    /// Get tunnel service id.
    fn id(&self) -> &str {
        &self.name
    }

    fn sample(&self) -> Sample {
        panic!("Unsupport sample")
    }
}

/// Create an channel to receive rhs tunnel.
pub fn make_tunnel_factory_channel<ID: ToString>(
    name: ID,
    buffer: usize,
) -> (TunnelFactorySender, TunnelFactoryReceiver) {
    let (sender, receiver) = mpsc::channel(buffer);

    (
        TunnelFactorySender {
            name: name.to_string(),
            sender,
        },
        receiver,
    )
}
