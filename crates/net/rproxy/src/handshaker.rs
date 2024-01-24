use std::io;

use async_trait::async_trait;
use bytes::BytesMut;
use futures::{
    channel::mpsc::{Receiver, Sender},
    Future,
};

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

// #[async_trait]
// impl<F> Handshaker for F
// where
//     F: Fn(HandshakeContext) -> io::Result<(HandshakeContext, TunnelOpenConfig)> + Send + Sync,
// {
//     async fn handshake(
//         &self,
//         cx: HandshakeContext,
//     ) -> io::Result<(HandshakeContext, TunnelOpenConfig)> {
//         self(cx)
//     }
// }

#[async_trait]
impl<F, Fut> Handshaker for F
where
    F: Fn(HandshakeContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = io::Result<(HandshakeContext, TunnelOpenConfig)>> + Send + Sync,
{
    async fn handshake(
        &self,
        cx: HandshakeContext,
    ) -> io::Result<(HandshakeContext, TunnelOpenConfig)> {
        self(cx).await
    }
}
