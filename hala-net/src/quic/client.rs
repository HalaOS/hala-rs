use futures::channel::mpsc::{Receiver, Sender};

use super::event::QuicEvent;

pub(crate) struct Incoming {
    pub(crate) id: quiche::ConnectionId<'static>,
    /// Quic connection instance.
    pub(crate) conn: quiche::Connection,
    /// Quic connection recv data channel
    pub(crate) data_receiver: Receiver<QuicEvent>,
    /// Quic connection send data channel
    pub(crate) data_sender: Sender<QuicEvent>,
}
