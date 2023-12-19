use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
    sync::Arc,
    time::Duration,
};

use hala_io_driver::Handle;
use hala_io_util::*;
use quiche::{RecvInfo, SendInfo};
use rand::seq::IteratorRandom;
use ring::rand::{SecureRandom, SystemRandom};

use crate::{errors::into_io_error, UdpGroup};

use super::{Config, QuicConn, QuicConnState};

/// Quic client connector
pub(crate) struct InnerConnector {
    /// source connection id.
    pub(super) quiche_conn: quiche::Connection,
}

impl InnerConnector {
    /// Create new quic connector
    pub(crate) fn new(
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
    pub(crate) fn send(&mut self, buf: &mut [u8]) -> io::Result<(usize, SendInfo)> {
        self.quiche_conn.send(buf).map_err(|err| {
            if err == quiche::Error::Done {
                io::Error::new(io::ErrorKind::TimedOut, err)
            } else {
                io::Error::new(io::ErrorKind::ConnectionRefused, err)
            }
        })
    }

    /// Accept remote peer data.
    pub(crate) fn recv(&mut self, buf: &mut [u8], recv_info: RecvInfo) -> io::Result<usize> {
        let len = self
            .quiche_conn
            .recv(buf, recv_info)
            .map_err(|err| io::Error::new(io::ErrorKind::ConnectionRefused, err))?;

        if self.quiche_conn.is_closed() {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                "Early stage reject",
            ));
        }

        Ok(len)
    }

    /// Check if underly connection is established.
    pub(crate) fn is_established(&self) -> bool {
        self.quiche_conn.is_established()
    }

    /// Returns the amount of time until the next timeout event.
    ///
    /// Once the given duration has elapsed, the [`on_timeout()`] method should
    /// be called. A timeout of `None` means that the timer should be disarmed.
    ///
    pub(crate) fn timeout(&self) -> Option<Duration> {
        self.quiche_conn.timeout()
    }

    /// Processes a timeout event.
    ///
    /// If no timeout has occurred it returns 0.
    pub(crate) fn on_timeout(&mut self) {
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
    udp_group: Arc<UdpGroup>,
}

impl QuicConnector {
    /// Create a new connector and bind to `laddrs`
    pub fn bind<L: ToSocketAddrs>(laddrs: L, config: Config) -> io::Result<Self> {
        Self::bind_with(laddrs, config, get_poller()?)
    }

    /// Create a new connector and bind to `laddrs`
    pub fn bind_with<L: ToSocketAddrs>(
        laddrs: L,
        config: Config,
        poller: Handle,
    ) -> io::Result<Self> {
        let udp_group = UdpGroup::bind_with(laddrs, poller)?;

        Ok(Self {
            udp_group: Arc::new(udp_group),
            config,
        })
    }

    /// Connect to remote addrs.
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
            let connector = match InnerConnector::new(&mut self.config, laddr.clone(), raddr) {
                Ok(c) => c,
                Err(err) => {
                    log::error!("Create connector, raddr={}, error={}", raddr, err);
                    last_error = Some(err);
                    continue;
                }
            };

            match self.connect_once(connector).await {
                Ok(conn) => {
                    let event_loop = QuicConnEventLoop {
                        conn: conn.clone(),
                        udp_group: self.udp_group.clone(),
                    };

                    event_loop.run_loop()?;

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
        let mut buf = [0; 65535];

        loop {
            let (send_size, send_info) = connector.send(&mut buf)?;

            self.udp_group
                .send_to(&buf[..send_size], send_info.to)
                .await?;

            let recv_timeout = connector.timeout();

            let (laddr, read_size, raddr) =
                match timeout(self.udp_group.recv_from(&mut buf), recv_timeout).await {
                    Ok(r) => r,
                    Err(err) if err.kind() == io::ErrorKind::TimedOut => {
                        log::trace!("connector={} timeout", connector.quiche_conn.trace_id());
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

#[derive(Clone)]
pub(super) struct QuicConnEventLoop {
    pub(super) conn: QuicConn,
    pub(super) udp_group: Arc<UdpGroup>,
}

impl QuicConnEventLoop {
    fn run_loop(&self) -> io::Result<()> {
        let clonsed = self.clone();

        io_spawn(async move { clonsed.clone().recv_loop().await })?;

        let clonsed = self.clone();

        io_spawn(async move { clonsed.clone().send_loop().await })?;

        Ok(())
    }

    async fn recv_loop(&self) -> io::Result<()> {
        let mut buf = [0; 65535];

        loop {
            let (laddr, read_size, raddr) = self.udp_group.recv_from(&mut buf).await?;

            let recv_info = RecvInfo {
                from: raddr,
                to: laddr,
            };

            let mut start_offset = 0;

            let end_offset = read_size;

            loop {
                let read_size = self
                    .conn
                    .state
                    .recv(&mut buf[start_offset..end_offset], recv_info)
                    .await?;

                start_offset += read_size;

                if start_offset == end_offset {
                    break;
                }
            }
        }
    }

    pub(super) async fn send_loop(&self) -> io::Result<()> {
        let mut buf = [0; 65535];

        loop {
            let (send_size, send_info) = match self.conn.state.send(&mut buf).await {
                Ok(r) => r,
                Err(err) => {
                    log::error!(
                        "Stop send_loop, conn={}, err={}",
                        self.conn.state.trace_id,
                        err
                    );

                    return Ok(());
                }
            };

            log::trace!(
                "Quiconn({:?}) send_size={}, send_info={:?}",
                self.conn,
                send_size,
                send_info
            );

            self.udp_group
                .send_to_on_path(&buf[..send_size], send_info.from, send_info.to)
                .await?;
        }
    }
}
