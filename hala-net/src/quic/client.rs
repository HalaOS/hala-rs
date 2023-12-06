use futures::channel::mpsc::{Receiver, Sender};

use super::{event::QuicEvent, Incoming};

pub struct QuicConn {
    id: quiche::ConnectionId<'static>,
    /// Quic connection instance.
    conn: quiche::Connection,
    /// Quic connection recv data channel
    data_receiver: Receiver<QuicEvent>,
    /// Quic connection send data channel
    data_sender: Sender<QuicEvent>,
}

impl From<Incoming> for QuicConn {
    fn from(value: Incoming) -> Self {
        Self {
            id: value.id,
            conn: value.conn,
            data_receiver: value.data_receiver,
            data_sender: value.data_sender,
        }
    }
}
