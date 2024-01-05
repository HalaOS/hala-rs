use std::{io, net::SocketAddr, time::Duration};

use quiche::{RecvInfo, SendInfo};
use ring::rand::{SecureRandom, SystemRandom};

use crate::{errors::into_io_error, Config};

/// Quic client connector
pub struct QuicConnectorState {
    /// source connection id.
    pub(super) quiche_conn: quiche::Connection,
}

impl QuicConnectorState {
    /// Create new quic connector
    pub fn new(
        config: &mut Config,
        laddr: SocketAddr,
        raddr: SocketAddr,
    ) -> io::Result<QuicConnectorState> {
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
