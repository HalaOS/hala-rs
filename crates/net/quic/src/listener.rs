use std::{io, net::ToSocketAddrs};

use hala_udp::UdpSocket;

use crate::{state::QuicListenerState, Config, QuicConn};

pub struct QuicListener {
    state: QuicListenerState,
}

impl QuicListener {
    pub fn bind<L: ToSocketAddrs>(laddrs: L, config: Config) -> io::Result<Self> {
        let udp_socket = UdpSocket::bind(laddrs)?;

        let max_datagram_size = config.max_datagram_size;

        let state = QuicListenerState::new(config)?;

        event_loop::run_event_loop(udp_socket, state.clone(), max_datagram_size);

        Ok(QuicListener { state })
    }

    /// Accept one incoming quic [`connection`](QuicConn).
    ///
    /// return [`None`], if this listener is been closed.
    pub async fn accept(&self) -> Option<QuicConn> {
        if let Some(state) = self.state.accept().await {
            Some(state.into())
        } else {
            None
        }
    }
}

mod event_loop {
    use std::sync::Arc;

    use hala_future::executor::spawn;
    use quiche::RecvInfo;

    use super::*;

    pub(super) fn run_event_loop(
        udp_socket: UdpSocket,
        state: QuicListenerState,
        max_datagram_size: usize,
    ) {
        let udp_socket = Arc::new(udp_socket);

        let udp_socket_cloned = udp_socket.clone();

        let state_cloned: QuicListenerState = state.clone();

        spawn(async move {
            match run_recv_event_loop(udp_socket_cloned, state_cloned, max_datagram_size).await {
                Ok(_) => {
                    log::trace!("quic listener recv loop stop.",);
                }
                Err(err) => {
                    log::trace!("quic listener recv loop stop with err, {}", err);
                }
            }
        });

        spawn(async move {
            match run_send_event_loop(udp_socket, state).await {
                Ok(_) => {
                    log::trace!("quic listener send loop stop.",);
                }
                Err(err) => {
                    log::trace!("quic listener send loop stop with err, {}", err);
                }
            }
        });
    }

    async fn run_recv_event_loop(
        udp_socket: Arc<UdpSocket>,
        state: QuicListenerState,
        max_datagram_size: usize,
    ) -> io::Result<()> {
        let mut buf = vec![0; max_datagram_size];

        let laddr = udp_socket.local_addr()?;

        loop {
            let (recv_size, raddr) = udp_socket.recv_from(&mut buf).await?;

            state
                .write(
                    buf.as_mut_slice(),
                    recv_size,
                    RecvInfo {
                        from: raddr,
                        to: laddr,
                    },
                )
                .await?;
        }
    }

    async fn run_send_event_loop(
        udp_socket: Arc<UdpSocket>,
        state: QuicListenerState,
    ) -> io::Result<()> {
        loop {
            let (buf, send_info) = state.read().await?;

            udp_socket.send_to(&buf, send_info.to).await?;
        }
    }
}
