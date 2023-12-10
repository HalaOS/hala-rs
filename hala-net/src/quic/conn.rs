use std::{io, net::ToSocketAddrs};

use super::inner_conn::QuicInnerConn;

/// Quic connection between a local and a remote.
#[derive(Clone)]
pub struct QuicConn {
    #[allow(unused)]
    inner: QuicInnerConn,
}

impl QuicConn {
    /// Connect to remote peer.
    pub async fn connect<R: ToSocketAddrs>(_raddrs: R) -> io::Result<Self> {
        todo!()
    }
}
