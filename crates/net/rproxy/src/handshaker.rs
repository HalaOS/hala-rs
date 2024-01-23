use std::io;

use async_trait::async_trait;
use bytes::BytesMut;
use futures::channel::mpsc::{Receiver, Sender};

use crate::protocol::{PathInfo, TransportConfig};

/// Reverse proxy handshake protocol must implement this trait.
///
#[async_trait]
pub trait Handshaker {
    /// Invoke handshake process and returns tunnel open configuration.
    async fn handshake(
        &self,
        cx: HandshakeContext,
    ) -> io::Result<(HandshakeContext, TunnelOpenConfig)>;
}

/// An owned dynamically typed [`Handshaker`].
pub type BoxHandshaker = Box<dyn Handshaker + Sync + Send + 'static>;

/// Handshake context data.
pub struct HandshakeContext {
    /// The path information of gateway transfer data.
    pub path: PathInfo,
    /// The max len for packet transfering with this tunnel.
    pub max_packet_len: usize,
    /// The max cache len of gateway transfer data.
    pub max_cache_len: usize,
    /// Backward data sender.
    pub backward: Sender<BytesMut>,
    /// Forward data receiver.
    pub forward: Receiver<BytesMut>,
}

/// Tunnel open configuration.
pub struct TunnelOpenConfig {
    pub max_packet_len: usize,
    /// The max cache len of gateway transfer data.
    pub max_cache_len: usize,
    /// The ID of tunnel for data forwarding.
    pub tunnel_service_id: String,
    /// The transport configuration for tunnel.
    pub transport_config: TransportConfig,
}
