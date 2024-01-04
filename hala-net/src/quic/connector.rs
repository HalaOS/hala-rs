use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
    rc::Rc,
    time::Duration,
};

use hala_io_driver::Handle;
use hala_io_util::*;
use quiche::{RecvInfo, SendInfo};
use rand::seq::IteratorRandom;
use ring::rand::{SecureRandom, SystemRandom};

use crate::{errors::into_io_error, UdpGroup};

use super::{eventloop::QuicConnEventLoop, Config, QuicConn, QuicConnState};

/// Quic client connector
pub struct InnerConnector {
    /// source connection id.
    pub(super) quiche_conn: quiche::Connection,
}

impl InnerConnector {
    /// Create new quic connector
    pub fn new(
        config: &mut Config,
        laddr: SocketAddr,
        raddr: SocketAddr,
    ) -> io::Result<InnerConnector> {
        let mut scid = vec![0; quiche::MAX_CONN_ID_LEN];

        SystemRandom::new().fill(&mut scid).map_err(into_io_error)?;

        let scid = quiche::ConnectionId::from_vec(scid);

        log::trace!("Connector {:?}", scid);

        let quiche_conn = quiche::connect(None, &scid, laddr, raddr, config)
            .map_err(|err| io::Error::new(io::ErrorKind::ConnectionRefused, err))?;

        Ok(Self { quiche_conn })
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

impl From<InnerConnector> for QuicConn {
    fn from(value: InnerConnector) -> Self {
        QuicConn::new(QuicConnState::new(value.quiche_conn, 4))
    }
}

/// Quic client connector instance.
pub struct QuicConnector {
    config: Config,
    udp_group: Rc<UdpGroup>,
}

impl QuicConnector {
    /// Create a new connector and bind to `laddrs`
    pub fn bind<L: ToSocketAddrs>(laddrs: L, config: Config) -> io::Result<Self> {
        Self::bind_with(laddrs, config, get_local_poller()?)
    }

    /// Create a new connector and bind to `laddrs`
    pub fn bind_with<L: ToSocketAddrs>(
        laddrs: L,
        config: Config,
        poller: Handle,
    ) -> io::Result<Self> {
        let udp_group = UdpGroup::bind_with(laddrs, poller)?;

        Ok(Self {
            udp_group: Rc::new(udp_group),
            config,
        })
    }

    /// Connect to remote addrs.
    #[inline]
    pub async fn connect<R>(&mut self, raddrs: R) -> io::Result<QuicConn>
    where
        R: ToSocketAddrs,
    {
        let laddr = self
            .udp_group
            .local_addrs()
            .choose(&mut rand::thread_rng())
            .unwrap()
            .clone();

        let mut last_error = None;

        for raddr in raddrs.to_socket_addrs()? {
            let connector = match InnerConnector::new(&mut self.config, laddr, raddr) {
                Ok(c) => c,
                Err(err) => {
                    log::error!("Create connector, raddr={}, error={}", raddr, err);
                    last_error = Some(err);
                    continue;
                }
            };

            match self.connect_once(connector).await {
                Ok(conn) => {
                    QuicConnEventLoop::client_event_loop(conn.clone(), self.udp_group.clone())?;

                    return Ok(conn);
                }
                Err(err) => {
                    last_error = Some(err);
                    continue;
                }
            }
        }

        return Err(last_error.unwrap());
    }

    async fn connect_once(&mut self, mut connector: InnerConnector) -> io::Result<QuicConn> {
        let mut buf = vec![0; 65535];

        loop {
            if let Some((send_size, send_info)) = connector.send(&mut buf)? {
                self.udp_group
                    .send_to(&buf[..send_size], send_info.to)
                    .await?;
            }

            let recv_timeout = connector.timeout();

            let (laddr, read_size, raddr) = match timeout_with(
                self.udp_group.recv_from(&mut buf),
                recv_timeout,
                get_local_poller()?,
            )
            .await
            {
                Ok(r) => r,
                Err(err) if err.kind() == io::ErrorKind::TimedOut => {
                    log::error!(
                        "connector={} timeout, duration={:?}",
                        connector.quiche_conn.trace_id(),
                        recv_timeout
                    );
                    // generate timeout retry package
                    connector.on_timeout();
                    continue;
                }
                Err(err) => {
                    return Err(err);
                }
            };

            connector.recv(
                &mut buf[..read_size],
                RecvInfo {
                    from: raddr,
                    to: laddr,
                },
            )?;

            if connector.is_established() {
                return Ok(connector.into());
            }
        }
    }
}
