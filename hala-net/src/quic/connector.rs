use std::{io, net::SocketAddr};

use quiche::{RecvInfo, SendInfo};
use ring::rand::{SecureRandom, SystemRandom};

use crate::errors::into_io_error;

use super::Config;

/// Quic client connector
pub struct Connector {
    /// source connection id.
    quiche_conn: quiche::Connection,
}

impl Connector {
    /// Create new quic connector
    pub fn new(mut config: Config, laddr: SocketAddr, raddr: SocketAddr) -> io::Result<Connector> {
        let mut scid = vec![0; quiche::MAX_CONN_ID_LEN];

        SystemRandom::new().fill(&mut scid).map_err(into_io_error)?;

        let scid = quiche::ConnectionId::from_vec(scid);

        let quiche_conn = quiche::connect(None, &scid, laddr, raddr, &mut config)
            .map_err(|err| io::Error::new(io::ErrorKind::ConnectionRefused, err))?;

        Ok(Self { quiche_conn })
    }

    ///
    pub fn send(&mut self, buf: &mut [u8]) -> io::Result<(usize, SendInfo)> {
        self.quiche_conn
            .send(buf)
            .map_err(|err| io::Error::new(io::ErrorKind::ConnectionRefused, err))
    }

    pub fn recv(&mut self, buf: &mut [u8], recv_info: RecvInfo) -> io::Result<(usize, bool)> {
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

        if self.quiche_conn.is_established() {
            Ok((len, true))
        } else {
            Ok((len, false))
        }
    }
}

impl From<Connector> for quiche::Connection {
    fn from(value: Connector) -> Self {
        value.quiche_conn
    }
}
