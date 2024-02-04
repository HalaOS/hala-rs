use std::io;

use async_trait::async_trait;
use hala_rs::{
    rproxy::{HandshakeContext, Handshaker, TransportConfig, TunnelOpenConfig},
    tls::{SslConnector, SslMethod},
};

use crate::ReverseProxy;

pub struct TcpHandshaker {
    config: ReverseProxy,
}

impl TcpHandshaker {
    /// Create new quic tunnel handshaker.
    pub fn new(config: ReverseProxy) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Handshaker for TcpHandshaker {
    async fn handshake(&self, cx: HandshakeContext) -> io::Result<TunnelOpenConfig> {
        let raddrs = self.config.raddrs.clone();

        let config = TunnelOpenConfig {
            session_id: cx.session_id,
            max_cache_len: self.config.max_cache_len,
            max_packet_len: self.config.max_packet_len,
            tunnel_service_id: "TcpTunnel".into(),
            transport_config: TransportConfig::Tcp(raddrs),
            gateway_path_info: cx.path,
            gateway_backward: cx.backward,
            gateway_forward: cx.forward,
        };

        Ok(config)
    }
}

pub struct TcpSslHandshaker {
    config: ReverseProxy,
}

impl TcpSslHandshaker {
    /// Create new quic tunnel handshaker.
    pub fn new(config: ReverseProxy) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Handshaker for TcpSslHandshaker {
    async fn handshake(&self, cx: HandshakeContext) -> io::Result<TunnelOpenConfig> {
        let raddrs = self.config.raddrs.clone();

        let tunnel_ca_file = self.config.tunnel_ca_file.clone();

        let domain = self
            .config
            .domain
            .clone()
            .expect("Tunnel TcpSsl that require provide the `domain` name of peer");

        let mut config = SslConnector::builder(SslMethod::tls()).unwrap();

        if self.config.verify_server {
            config
                .set_ca_file(
                    tunnel_ca_file
                        .expect("Tunnel verify_server is on that require provide tunnel_ca_file"),
                )
                .unwrap();
        }

        let config = config.build().configure().unwrap();

        let config = TunnelOpenConfig {
            session_id: cx.session_id,
            max_cache_len: self.config.max_cache_len,
            max_packet_len: self.config.max_packet_len,
            tunnel_service_id: "TcpSslTunnel".into(),
            transport_config: TransportConfig::Ssl {
                raddrs,
                domain,
                config,
            },
            gateway_path_info: cx.path,
            gateway_backward: cx.backward,
            gateway_forward: cx.forward,
        };

        Ok(config)
    }
}
