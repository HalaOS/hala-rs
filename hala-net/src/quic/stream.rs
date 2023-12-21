use std::{fmt::Debug, io, sync::Arc};

use futures::{AsyncRead, AsyncWrite, FutureExt};

use super::AsyncQuicConnState;

#[derive(Clone)]
pub struct QuicStream {
    stream_id: Arc<u64>,
    state: AsyncQuicConnState,
}

impl Debug for QuicStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "conn={}, stream_id={}",
            self.state.trace_id, self.stream_id
        )
    }
}

impl QuicStream {
    pub(super) fn new(stream_id: u64, state: AsyncQuicConnState) -> Self {
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

    pub async fn is_closed(&self) -> bool {
        self.state.is_stream_closed(*self.stream_id).await
    }
}

impl Drop for QuicStream {
    fn drop(&mut self) {
        if Arc::strong_count(&self.stream_id) == 1 {
            log::trace!(
                "drop stream conn={}, stream_id={}",
                self.state.trace_id,
                self.stream_id
            );
            self.state.close_stream(*self.stream_id);
        }
    }
}

impl AsyncWrite for QuicStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        Box::pin(self.state.stream_send(*self.stream_id, buf, false)).poll_unpin(cx)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        Box::pin(self.state.stream_send(*self.stream_id, b"", true))
            .poll_unpin(cx)
            .map(|_| Ok(()))
    }
}

impl AsyncRead for QuicStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<io::Result<usize>> {
        Box::pin(self.state.stream_recv(*self.stream_id, buf))
            .poll_unpin(cx)
            .map(|r| r.map(|(read_size, _)| read_size))
    }
}
