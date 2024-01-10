use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
};

use hala_udp::UdpSocket;

use crate::{state::QuicListenerState, Config, QuicConn};

pub struct QuicListener {
    state: QuicListenerState,
    laddr: SocketAddr,
}

impl QuicListener {
    /// Create new quic server listener and bind to `laddrs`.
    pub fn bind<L: ToSocketAddrs>(laddrs: L, config: Config) -> io::Result<Self> {
        let udp_socket = UdpSocket::bind(laddrs)?;

        let laddr = udp_socket.local_addr()?;

        let max_datagram_size = config.max_datagram_size;

        let state = QuicListenerState::new(config)?;

        event_loop::run_event_loop(udp_socket, state.clone(), max_datagram_size);

        Ok(QuicListener { state, laddr })
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

    /// Get the [`SocketAddr`] to which this listener is bound.
    pub fn local_addr(&self) -> SocketAddr {
        self.laddr
    }
}

mod event_loop {
    use std::sync::Arc;

    use hala_future::executor::future_spawn;
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

        future_spawn(async move {
            match run_recv_event_loop(udp_socket_cloned, state_cloned, max_datagram_size).await {
                Ok(_) => {
                    log::trace!("quic listener recv loop stop.",);
                }
                Err(err) => {
                    log::trace!("quic listener recv loop stop with err, {}", err);
                }
            }
        });

        future_spawn(async move {
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

        log::trace!("Quic server listen on {:?}, start recv data.", laddr);

        loop {
            let (recv_size, raddr) = udp_socket.recv_from(&mut buf).await?;

            log::trace!("Quic server recv data, len={}, raddr={}", recv_size, raddr);

            let write_result = state
                .write(
                    buf.as_mut_slice(),
                    recv_size,
                    RecvInfo {
                        from: raddr,
                        to: laddr,
                    },
                )
                .await?;

            match write_result {
                crate::state::QuicListenerWriteResult::Internal {
                    write_size: _,
                    read_size,
                    send_info,
                } => {
                    if read_size > 0 {
                        log::trace!(
                            "Quic server forward internal packet data, len={}, raddr={}",
                            recv_size,
                            send_info.to
                        );

                        udp_socket
                            .send_to(&mut buf[..read_size], send_info.to)
                            .await?;
                    }
                }
                _ => {}
            }
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
