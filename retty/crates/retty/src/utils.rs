use hala_rs::net::quic::QuicConnectionId;
use multiaddr::Multiaddr;
use uuid::Uuid;

/// The connection path of transport layer
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConnPath {
    /// The connection from endpoint.
    pub from: Multiaddr,
    /// The connection to endpoint
    pub to: Multiaddr,
}

/// The connection id of transport layer.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConnId<'a> {
    /// Connection id for tcp.
    Tcp(Uuid),
    /// Connection id for quic stream.
    QuicStream(QuicConnectionId<'a>, u64),
}

impl<'a> ConnId<'a> {
    /// Consume self and return an owned version [`ConnId`] instance.
    #[inline]
    pub fn into_owned(self) -> ConnId<'static> {
        match self {
            ConnId::Tcp(uuid) => ConnId::Tcp(uuid),
            ConnId::QuicStream(cid, stream_id) => ConnId::QuicStream(cid.into_owned(), stream_id),
        }
    }
}
