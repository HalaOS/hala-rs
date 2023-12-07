use std::{io, net::ToSocketAddrs};

use futures::{channel::mpsc::Receiver, StreamExt};

use ring::rand::SystemRandom;

use crate::UdpGroup;

use super::{Config, QuicConn, QuicServerEventLoop};

/// A Quic server, listening for connections.
pub struct QuicListener {
    /// New connection receiver.
    incoming_receiver: Receiver<QuicConn>,
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

        let rng = SystemRandom::new();

        let conn_id_seed = ring::hmac::Key::generate(ring::hmac::HMAC_SHA256, &rng)
            .map_err(|err| io::Error::new(io::ErrorKind::Other, format!("${err}")))?;

        let event_loop = QuicServerEventLoop::new(
            config,
            udp_data_receiver,
            incoming_sender,
            udp_group,
            udp_data_sender,
            conn_id_seed,
        );

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
