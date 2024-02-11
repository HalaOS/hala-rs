use std::{fmt::Debug, net::SocketAddr};

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
    Forward(Uuid, usize),
    Backward(Uuid, usize),
}

impl ProfileEvent {
    pub fn connect(laddr: SocketAddr, raddr: SocketAddr) -> (Uuid, Self) {
        let uuid = Uuid::new_v4();

        (
            uuid,
            ProfileEvent::Connect(Box::new(ProfileConnect { uuid, laddr, raddr })),
        )
    }

    pub fn prohibited(uuid: Uuid) -> Self {
        ProfileEvent::Prohibited(uuid)
    }
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
}

def_target!(GATEWAY_EVENT, "Gateway profiling event");
def_target!(TUNNEL_EVENT, "Tunnel profiling event");
