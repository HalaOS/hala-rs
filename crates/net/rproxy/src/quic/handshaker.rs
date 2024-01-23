use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
};

use async_trait::async_trait;
use hala_quic::Config;

use crate::{HandshakeContext, Handshaker, TunnelOpenConfig};

/// The handshaker of unconditional forwarding data with quic tunnel
pub struct QuicTunnelForwarding<F> {
    config_maker: F,
    max_cache_len: usize,
    tunnel_factory_id: String,
    raddrs: Vec<SocketAddr>,
}

impl<F> QuicTunnelForwarding<F> {
    /// Create `QuicTunnelForwarding` with a default configuration other than the [`config`](hala_quic::Config) `maker` and the `raddrs`.
    pub fn new<R: ToSocketAddrs>(raddrs: R, config_maker: F) -> io::Result<Self> {
        let raddrs = raddrs.to_socket_addrs()?.collect::<Vec<_>>();

        Ok(Self {
            config_maker,
            max_cache_len: 1024,
            tunnel_factory_id: "QuicTunnel".into(),
            raddrs,
        })
    }

    /// Consume self and set the tunnel `max_cache_len` config.
    pub fn set_max_cache_len(mut self, value: usize) -> Self {
        self.max_cache_len = value;
        self
    }

    /// Consume self and set the tunnel `tunnel_factory_id` config.
    pub fn set_tunnel_factory_id<ID: ToString>(mut self, value: ID) -> Self {
        self.tunnel_factory_id = value.to_string();
        self
    }
}

#[async_trait]
impl<F> Handshaker for QuicTunnelForwarding<F>
where
    F: Fn() -> Config + Send + Sync + 'static,
{
    /// Invoke handshake process and returns tunnel open configuration.
    async fn handshake(
        &self,
        cx: HandshakeContext,
    ) -> io::Result<(HandshakeContext, TunnelOpenConfig)> {
        let quic_config = (self.config_maker)();

        let tunnel_open_config = TunnelOpenConfig {
            max_cache_len: self.max_cache_len,
            max_packet_len: quic_config.max_datagram_size,
            tunnel_service_id: self.tunnel_factory_id.clone(),
            transport_config: crate::TransportConfig::Quic(self.raddrs.clone(), quic_config),
        };

        Ok((cx, tunnel_open_config))
    }
}

#[cfg(test)]
mod tests {
    use hala_quic::Config;

    use crate::TunnelFactoryManager;

    use super::QuicTunnelForwarding;

    fn mock_config() -> Config {
        Config::new().unwrap()
    }

    #[test]
    fn test_create_tunnel_factory_manager() {
        let quic_tunnel_forwarding = QuicTunnelForwarding::new("127.0.0:0", mock_config).unwrap();

        TunnelFactoryManager::new(quic_tunnel_forwarding);
    }
}
