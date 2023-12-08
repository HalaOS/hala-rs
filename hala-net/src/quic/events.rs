use bytes::Bytes;
use futures::channel::mpsc::Sender;
use quiche::ConnectionId;

pub(crate) enum CloseEvent {
    Connection(ConnectionId<'static>),
    Stream {
        conn_id: ConnectionId<'static>,
        stream_id: u64,
    },
}

/// Events passed between `QuicConn` and `QuicServerEventLoop` / `QuicClientEventLoop`
pub(crate) enum QuicConnEvent {
    OpenStream {
        /// Connection ID of this stream
        conn_id: ConnectionId<'static>,
        /// New stream id
        stream_id: u64,
        /// Stream event sender
        sender: Sender<QuicConnEvent>,
    },
    StreamData {
        bytes: Bytes,
        fin: bool,
    },
}
