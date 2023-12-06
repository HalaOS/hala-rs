use std::{collections::HashMap, io, net::ToSocketAddrs, sync::Arc};

use futures::{
    channel::mpsc::{Receiver, Sender},
    SinkExt, StreamExt,
};

use crate::UdpGroup;

use super::{Config, QuicConn, QuicEvent};

pub(crate) struct Incoming {
    pub(crate) id: quiche::ConnectionId<'static>,
    /// Quic connection instance.
    pub(crate) conn: quiche::Connection,
    /// Quic connection recv data channel
    pub(crate) data_receiver: Receiver<QuicEvent>,
    /// Quic connection send data channel
    pub(crate) data_sender: Sender<QuicEvent>,
}

struct QuicConnProxy {
    /// Quic connection recv data channel
    data_sender: Sender<QuicEvent>,
}

/// A Quic server, listening for connections.
pub struct QuicListener {
    /// New connection receiver.
    incoming_receiver: Receiver<Incoming>,
}

impl QuicListener {
    pub fn bind<S: ToSocketAddrs>(
        laddrs: S,
        config: Config,
    ) -> io::Result<(Self, QuicServerEventLoop)> {
        let udp_group = UdpGroup::bind(laddrs)?;

        let (incoming_sender, incoming_receiver) =
            futures::channel::mpsc::channel(config.incoming_conn_channel_len);

        let (udp_data_sender, udp_data_receiver) =
            futures::channel::mpsc::channel(config.udp_data_channel_len);

        let listener = Self { incoming_receiver };

        let event_loop = QuicServerEventLoop {
            incoming_sender,
            udp_data_receiver,
            udp_group,
            config,
            conns: Default::default(),
            udp_data_sender,
        };

        Ok((listener, event_loop))
    }

    /// Accept one Quic incoming connection.
    pub async fn accept(&mut self) -> io::Result<QuicConn> {
        self.incoming_receiver
            .next()
            .await
            .map(|incoming| incoming.into())
            .ok_or(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "QuicListener shutdown",
            ))
    }
}

pub struct QuicServerEventLoop {
    /// Mapping from ConnectionId => QuicConnProxy, shared with `QuicListener`
    conns: HashMap<quiche::ConnectionId<'static>, QuicConnProxy>,
    /// Quic server config
    config: Config,
    /// data receiver for which needs to be sent via udp_group to remote peers
    udp_data_receiver: Receiver<QuicEvent>,
    /// incoming connection sender
    incoming_sender: Sender<Incoming>,
    // Quice listener sockets
    udp_group: UdpGroup,
    /// data sender for which needs to be sent via udp_group to remote peers
    udp_data_sender: Sender<QuicEvent>,
}

impl QuicServerEventLoop {
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

    /// Run event loop
    pub async fn event_loop() {}
}
