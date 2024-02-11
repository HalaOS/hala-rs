use std::{fmt::Debug, net::SocketAddr, time::Instant};

use hala_pprof::def_target;
use hala_quic::QuicConnectionId;
use uuid::Uuid;

/// Profile event variant.
#[derive(Debug)]
pub enum ProfileEvent {
    /// Connection connected.
    Connect(Box<ProfileConnect>),
    /// Connection disconnected.
    Disconnect(Uuid),
    /// Prohibited connection or mux stream.
    Prohibited(Uuid),
    /// mux stream opened.
    OpenStream(Box<ProfileOpenStream>),
    /// mux stream closed.
    CloseStream(Uuid),
    /// Transmisson stats updated.
    Transport(Box<ProfileTransport>),
}

/// Profile event `Connect` content
#[derive(Debug)]
pub struct ProfileConnect {
    /// The unique id of this connection.
    pub uuid: Uuid,
    /// Connection local address.
    pub laddr: SocketAddr,
    /// Connection remote address.
    pub raddr: SocketAddr,
    /// Timestamp of this event.
    pub at: Instant,
}

/// Profile event `OpenStream` content.
#[derive(Debug)]
pub struct ProfileOpenStream {
    /// The unique id of quic stream.
    pub uuid: Uuid,
    /// Stream source id
    pub scid: QuicConnectionId<'static>,
    /// Stream destination id.
    pub dcid: QuicConnectionId<'static>,
    /// Stream id .
    pub stream_id: u64,
    /// Timestamp of this event.
    pub at: Instant,
}

/// Transmission statistics
#[derive(Debug)]
pub struct ProfileTransport {
    /// The unique id of this connection / stream.
    pub uuid: Uuid,
    /// forwarding data in bytes.
    pub forwarding_data: u64,
    /// backwarding data in bytes
    pub backwarding_data: u64,
    /// True if this is the last update for this connection's transport stats.
    pub is_final_update: bool,
    /// Timestamp of this event.
    pub at: Instant,
}

def_target!(GATEWAY_EVENT, "Gateway profiling event");
def_target!(TUNNEL_EVENT, "Tunnel profiling event");
