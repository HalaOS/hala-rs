use std::net::SocketAddr;

use super::MAX_DATAGRAM_SIZE;

/// Inner quic event variant.
pub(crate) enum QuicEvent {
    UdpData {
        buf: [u8; MAX_DATAGRAM_SIZE],
        data_len: usize,
        from: SocketAddr,
        to: SocketAddr,
    },
}
