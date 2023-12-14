use super::QuicConnState;

pub struct QuicStream {
    stream_id: u64,
    state: QuicConnState,
}

impl QuicStream {
    pub(super) fn new(stream_id: u64, state: QuicConnState) -> Self {
        Self { stream_id, state }
    }
}
