use std::{fmt::Debug, io};

use super::{QuicConnState, QuicStream};

/// Quic connection between a local and a remote.
#[derive(Clone)]
pub struct QuicConn {
    pub(super) state: QuicConnState,
}

impl Debug for QuicConn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "trace_id={}", self.state.trace_id)
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
        Ok(self.state.open_stream())
    }

    pub fn trace_id(&self) -> &str {
        &self.state.trace_id
    }
}
