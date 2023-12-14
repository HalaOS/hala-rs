use std::{io, sync::Arc};

use super::QuicConnState;

#[derive(Clone)]
pub struct QuicStream {
    stream_id: Arc<u64>,
    state: QuicConnState,
}

impl QuicStream {
    pub(super) fn new(stream_id: u64, state: QuicConnState) -> Self {
        Self {
            stream_id: Arc::new(stream_id),
            state,
        }
    }

    /// Create new future for send stream data
    pub async fn stream_send<'a>(&self, buf: &[u8], fin: bool) -> io::Result<usize> {
        self.state.stream_send(*self.stream_id, buf, fin).await
    }

    /// Create new future for recv stream data
    pub async fn stream_recv(&self, buf: &mut [u8]) -> io::Result<(usize, bool)> {
        self.state.stream_recv(*self.stream_id, buf).await
    }
}

impl Drop for QuicStream {
    fn drop(&mut self) {
        if Arc::strong_count(&self.stream_id) == 1 {
            self.state.close_stream(*self.stream_id);
        }
    }
}
