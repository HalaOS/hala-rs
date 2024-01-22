use std::net::SocketAddr;

use hala_quic::QuicConnectionId;

/// transport config data for reverse proxy.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TransportConfig {
    None,
    /// Tcp transport with bind socket address.
    Tcp(Vec<SocketAddr>),
    /// Quic transport with bind socket addresses and [`Config`].
    Quic(Vec<SocketAddr>, Vec<SocketAddr>),
    /// Tcp + SSL transport for client.
    Ssl(SocketAddr),
}

/// Transfer path information.
#[derive(Debug, Clone)]
pub enum PathInfo {
    None,
    /// Tcp transfer path info
    Tcp(SocketAddr, SocketAddr),
    /// Quic transfer path info
    Quic(QuicConnectionId<'static>, QuicConnectionId<'static>),
    /// Tcp + Ssl transfer path info
    Ssl(SocketAddr, SocketAddr),
}
