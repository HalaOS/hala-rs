use std::{collections::HashMap, io};

use futures::{
    channel::mpsc::{Receiver, Sender},
    SinkExt,
};

use super::{Incoming, QuicEvent};

struct QuicConnProxy {
    /// Quic connection recv data channel
    data_sender: Sender<QuicEvent>,
}

/// A Quic server, listening for connections.
pub struct QuicListener {
    conns: HashMap<quiche::ConnectionId<'static>, QuicConnProxy>,
    incoming_sender: Sender<Incoming>,
    /// New connection receiver.
    incoming_receiver: Receiver<Incoming>,
    /// data receiver for which needs to be sent via udp_group to remote peers
    udp_data_receiver: Receiver<QuicEvent>,
    /// data sender for which needs to be sent via udp_group to remote peers
    udp_data_sender: Sender<QuicEvent>,
}

impl QuicListener {
    /// Accept one incoming connection.595
    async fn on_incoming(
        &mut self,
        id: quiche::ConnectionId<'static>,
        conn: quiche::Connection,
    ) -> io::Result<()> {
        let (sender, receiver) = futures::channel::mpsc::channel(1024);

        let proxy = QuicConnProxy {
            data_sender: sender,
        };

        let incoming = Incoming {
            id: id.clone(),
            conn,
            data_receiver: receiver,
            data_sender: self.udp_data_sender.clone(),
        };

        self.conns.insert(id, proxy);

        self.incoming_sender
            .send(incoming)
            .await
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))
    }
}
