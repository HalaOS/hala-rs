use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
    sync::Arc,
    time::Duration,
};

use hala_future::executor::future_spawn;
use hala_io::timeout;
use hala_udp::UdpGroup;
use quiche::{ConnectionId, RecvInfo, SendInfo};
use rand::{seq::SliceRandom, thread_rng};
use ring::rand::{SecureRandom, SystemRandom};

use crate::{
    cs::{quic_conn_recv_loop, quic_conn_send_loop},
    errors::into_io_error,
    Config,
};

use super::QuicConn;

/// The factory to build [`QuicConnector`]
pub struct QuicConnectorBuilder {
    scid: ConnectionId<'static>,
    laddrs: Vec<SocketAddr>,
    raddrs: Vec<SocketAddr>,
    config: Config,
    offset: usize,
}

impl QuicConnectorBuilder {
    /// Create new quic connector
    pub fn new<L: ToSocketAddrs, R: ToSocketAddrs>(
        laddrs: L,
        raddrs: R,
        config: Config,
    ) -> io::Result<QuicConnectorBuilder> {
        let mut scid = vec![0; quiche::MAX_CONN_ID_LEN];

        SystemRandom::new().fill(&mut scid).map_err(into_io_error)?;

        let scid = quiche::ConnectionId::from_vec(scid);

        log::trace!("Connector {:?}", scid);

        let laddrs = laddrs.to_socket_addrs()?.collect::<Vec<_>>();
        let raddrs = raddrs.to_socket_addrs()?.collect::<Vec<_>>();

        Ok(Self {
            scid,
            laddrs,
            raddrs,
            config,
            offset: 0,
        })
    }

    /// Build next quic connector.
    pub fn build_next(&mut self) -> io::Result<Option<QuicConnector>> {
        if self.offset < self.raddrs.len() {
            let laddr = *self.laddrs.choose(&mut thread_rng()).unwrap();

            Ok(Some(QuicConnector::new(
                &self.scid,
                laddr,
                self.raddrs[self.offset],
                &mut self.config,
            )?))
        } else {
            Ok(None)
        }
    }
}

/// The context for quic client to connect to server.
pub struct QuicConnector {
    quiche_conn: quiche::Connection,
    max_datagram_size: usize,
    raddr: SocketAddr,
}

impl QuicConnector {
    /// Create new Quic connector.
    pub fn new(
        scid: &ConnectionId<'static>,
        laddr: SocketAddr,
        raddr: SocketAddr,
        config: &mut Config,
    ) -> io::Result<Self> {
        let quiche_conn =
            quiche::connect(None, scid, laddr, raddr, config).map_err(into_io_error)?;

        Ok(QuicConnector {
            quiche_conn,
            max_datagram_size: config.max_datagram_size,
            raddr,
        })
    }

    /// Generate send data.
    pub fn send(&mut self, buf: &mut [u8]) -> io::Result<Option<(usize, SendInfo)>> {
        match self.quiche_conn.send(buf) {
            Ok((send_size, send_info)) => {
                log::trace!(
                    "connector, id={:?}, send_size={}, send_info={:?}",
                    self.quiche_conn.source_id(),
                    send_size,
                    send_info
                );

                return Ok(Some((send_size, send_info)));
            }
            Err(err) if err == quiche::Error::Done => {
                log::trace!(
                    "connector, id={:?}, send done",
                    self.quiche_conn.source_id(),
                );

                return Ok(None);
            }
            Err(err) => {
                log::error!(
                    "connector, id={:?}, send err={}",
                    self.quiche_conn.source_id(),
                    err
                );
                return Err(io::Error::new(io::ErrorKind::ConnectionRefused, err));
            }
        }
    }

    /// Accept remote peer data.
    pub fn recv(&mut self, buf: &mut [u8], recv_info: RecvInfo) -> io::Result<usize> {
        let len = self.quiche_conn.recv(buf, recv_info).map_err(|err| {
            log::error!(
                "connector, id={:?}, recv err={}",
                self.quiche_conn.source_id(),
                err
            );

            return io::Error::new(io::ErrorKind::ConnectionRefused, err);
        })?;

        if self.quiche_conn.is_closed() {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                format!(
                    "Early stage reject, conn_id={:?}",
                    self.quiche_conn.source_id()
                ),
            ));
        }

        Ok(len)
    }

    /// Check if underly connection is established.
    pub fn is_established(&self) -> bool {
        self.quiche_conn.is_established()
    }

    /// Check if underly connection is closed.
    pub fn is_closed(&self) -> bool {
        self.quiche_conn.is_closed()
    }

    /// Returns the amount of time until the next timeout event.
    ///
    /// Once the given duration has elapsed, the [`on_timeout()`] method should
    /// be called. A timeout of `None` means that the timer should be disarmed.
    ///
    pub fn timeout(&self) -> Option<Duration> {
        self.quiche_conn.timeout()
    }

    /// Processes a timeout event.
    ///
    /// If no timeout has occurred it returns 0.
    pub fn on_timeout(&mut self) {
        self.quiche_conn.on_timeout();
    }
}

/// try to establish a new quic connection using the provided [`QuicConnectorBuilder`]
pub async fn connect<L: ToSocketAddrs, R: ToSocketAddrs>(
    mut builder: QuicConnectorBuilder,
) -> io::Result<QuicConn> {
    let udpsocket = UdpGroup::bind(builder.laddrs.as_slice())?;

    let mut lastest_error = None;

    while let Some(mut connector) = builder.build_next()? {
        match connect_with(&udpsocket, &mut connector).await {
            Err(err) => lastest_error = Some(err),
            Ok(_) => {
                let (conn, state, receiver) = QuicConn::make_client_conn(
                    connector.quiche_conn,
                    builder.config.send_ping_interval,
                    builder.config.max_conn_state_cache_len,
                );

                let udp_socket = Arc::new(udpsocket);

                state.start();

                future_spawn(quic_conn_send_loop(
                    format!("{:?}", conn),
                    receiver,
                    udp_socket.clone(),
                ));

                future_spawn(quic_conn_recv_loop(
                    format!("{:?}", conn),
                    conn.event_sender(),
                    udp_socket,
                ));

                return Ok(conn);
            }
        }
    }

    return Err(lastest_error.unwrap());
}

async fn connect_with(udp_socket: &UdpGroup, connector: &mut QuicConnector) -> io::Result<()> {
    let mut buf = vec![0; connector.max_datagram_size];

    loop {
        if let Some((send_size, send_info)) = connector.send(&mut buf)? {
            udp_socket
                .send_to_on_path(
                    &buf[..send_size],
                    hala_udp::PathInfo {
                        from: send_info.from,
                        to: send_info.to,
                    },
                )
                .await?;
        }

        if connector.is_closed() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "Connect to remove server timeout, raddr={:?}",
                    connector.raddr
                ),
            ));
        }

        let send_timeout = connector.timeout();

        log::trace!("Connect send timeout={:?}", send_timeout);

        let (recv_size, path_info) =
            match timeout(udp_socket.recv_from(&mut buf), send_timeout).await {
                Ok(r) => r,
                Err(err) if err.kind() == io::ErrorKind::TimedOut => {
                    connector.on_timeout();
                    continue;
                }
                Err(err) => return Err(err),
            };

        connector.recv(
            &mut buf[..recv_size],
            RecvInfo {
                from: path_info.from,
                to: path_info.to,
            },
        )?;

        if connector.is_established() {
            return Ok(());
        }
    }
}
