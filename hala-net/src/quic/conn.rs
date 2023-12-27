use std::{fmt::Debug, io};

use super::{QuicConnState, QuicStream};

/// Quic connection between a local and a remote.
#[derive(Clone)]
pub struct QuicConn {
    pub(super) state: QuicConnState,
}

impl Debug for QuicConn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "quic_conn={}", self.state.trace_id)
    }
}

impl QuicConn {
    pub(crate) fn new(state: QuicConnState) -> Self {
        Self { state }
    }
    /// Accept new incoming stream.
    pub async fn accept(&self) -> Option<QuicStream> {
        self.state.accept().await
    }

    /// Open new outgoing stream.
    pub async fn open_stream(&self) -> io::Result<QuicStream> {
        self.state.open_stream().await
    }

    /// Close current quic connection.
    pub async fn close(&self, app: bool, err: u64, reason: &[u8]) -> io::Result<()> {
        self.state.close(app, err, reason).await
    }

    /// Get `QuicConn` trace id &str
    pub fn trace_id(&self) -> &str {
        &self.state.trace_id
    }

    pub async fn is_closed(&self) -> bool {
        self.state.is_closed().await
    }
}
