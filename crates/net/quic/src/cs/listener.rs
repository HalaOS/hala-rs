use std::net::SocketAddr;
use std::sync::Arc;
use std::{io, net::ToSocketAddrs};

use futures::channel::mpsc;
use futures::channel::mpsc::Receiver;
use futures::StreamExt;
use hala_future::executor::future_spawn;
use hala_udp::UdpGroup;

use crate::cs::event_loop::{quic_conn_send_loop, quic_listener_recv_loop};
use crate::Config;

use super::QuicConn;
use super::QuicListenerState;

pub struct QuicListener {
    laddrs: Vec<SocketAddr>,
    incoming_conn_receiver: Receiver<QuicConn>,
}

impl QuicListener {
    pub fn bind<L: ToSocketAddrs>(laddrs: L, config: Config) -> io::Result<Self> {
        let udp_socket = Arc::new(UdpGroup::bind(laddrs)?);

        let laddrs = udp_socket
            .local_addrs()
            .map(|addr| *addr)
            .collect::<Vec<_>>();

        let (udp_data_sender, listener_data_receiver) =
            mpsc::channel(config.max_conn_state_cache_len);

        let (listener_data_sender, udp_data_receiver) =
            mpsc::channel(config.max_conn_state_cache_len);

        let (incoming_conn_sender, incoming_conn_receiver) =
            mpsc::channel(config.max_conn_state_cache_len);

        let state = QuicListenerState::new(
            "QuicListener".into(),
            config,
            listener_data_sender,
            listener_data_receiver,
            incoming_conn_sender,
        )?;

        state.start();

        future_spawn(quic_conn_send_loop(
            "QuicListener".into(),
            udp_data_receiver,
            udp_socket.clone(),
        ));

        future_spawn(quic_listener_recv_loop(
            "QuicListener".into(),
            udp_data_sender,
            udp_socket.clone(),
        ));

        Ok(Self {
            incoming_conn_receiver,
            laddrs,
        })
    }
}

impl QuicListener {
    /// Accept new incoming quic connection.
    pub async fn accept(&mut self) -> Option<QuicConn> {
        self.incoming_conn_receiver.next().await
    }

    /// Get the [`SocketAddr`] to which this listener is bound.
    pub fn local_addrs(&self) -> impl Iterator<Item = &SocketAddr> {
        self.laddrs.iter()
    }
}
