use std::net::SocketAddr;

use clap::ValueEnum;
use hala_quic::{Config, QuicConnectionId};
use hala_tls::{ConnectConfiguration, SslAcceptor};

/// transport config data for reverse proxy.
pub enum TransportConfig {
    /// empty transport config.
    None,
    /// Tcp transport with bind socket address.
    Tcp(Vec<SocketAddr>),
    /// Quic transport with bind socket addresses and [`Config`].
    Quic(Vec<SocketAddr>, Config),
    /// ssl client side transport config.
    Ssl {
        /// remote server address list.
        raddrs: Vec<SocketAddr>,
        /// remote peer domain
        domain: String,
        /// client connect config.
        config: ConnectConfiguration,
    },
    /// Ssl server side transport config.
    SslServer(Vec<SocketAddr>, SslAcceptor),
}

#[derive(ValueEnum, Clone, Debug)]
pub enum Protocol {
    // Http,
    // Https,
    TcpSsl,
    Tcp,
    Quic,
}

pub struct ProtocolConfig {
    /// The config for transport layer
    pub transport_config: TransportConfig,
    /// The max packet len for trasnferring.
    pub max_packet_len: usize,
    /// The max cached packet len for trasnferring.
    pub max_cache_len: usize,
}

/// Transfer path information.
#[derive(Debug, Clone)]
pub enum PathInfo {
    /// Empty path information.
    None,
    /// Tcp transfer path info
    Tcp(SocketAddr, SocketAddr),
    /// Quic transfer path info
    Quic(QuicConnectionId<'static>, QuicConnectionId<'static>),
    /// Tcp + Ssl transfer path info
    Ssl(SocketAddr, SocketAddr),
}
