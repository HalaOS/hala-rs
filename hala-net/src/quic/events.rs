use bytes::Bytes;
use futures::channel::mpsc::Sender;
use quiche::ConnectionId;

pub(crate) enum CloseEvent {
    Connection(ConnectionId<'static>),
    #[allow(unused)]
    Stream {
        conn_id: ConnectionId<'static>,
        stream_id: u64,
    },
}

/// Events passed between `QuicConn` and `QuicServerEventLoop` / `QuicClientEventLoop`
#[allow(unused)]
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
        /// Connection ID of this stream
        conn_id: ConnectionId<'static>,
        /// New stream id
        stream_id: u64,
        /// Data
        bytes: Bytes,
        fin: bool,
    },
}
