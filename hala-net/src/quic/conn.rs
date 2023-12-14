use std::io;

use super::{QuicConnState, QuicStream};

/// Quic connection between a local and a remote.
#[derive(Clone)]
pub struct QuicConn {
    pub(super) state: QuicConnState,
}

impl QuicConn {
    pub(crate) fn new(state: QuicConnState) -> Self {
        Self { state }
    }
    /// Accept new incoming stream.
    pub async fn accept(&self) -> io::Result<QuicStream> {
        self.state.accept().await
    }

    /// Open new outgoing stream.
    pub async fn open_stream(&self) -> io::Result<QuicStream> {
        Ok(self.state.open_stream())
    }
}
