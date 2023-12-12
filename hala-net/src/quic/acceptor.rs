use std::{collections::HashMap, io, net::SocketAddr, time::Duration};

use bytes::BytesMut;
use hala_io_util::ReadBuf;
use quiche::{ConnectionId, Header};
use ring::{hmac::Key, rand::SystemRandom};

use crate::errors::into_io_error;

use super::{inner_conn::QuicInnerConn, Config, MAX_DATAGRAM_SIZE};

pub struct ServerHello {
    pub read_size: usize,
    pub write_size: usize,
    pub conn: Option<QuicInnerConn>,
}

pub enum Accept<'a> {
    Handling(BytesMut),

    HandlingWithTimeout {
        conn_id: ConnectionId<'static>,
        send_buf: BytesMut,
        timeout: Option<Duration>,
    },
    Bypass(Header<'a>),
    Incoming(QuicInnerConn),
}

/// Quic server connection acceptor
pub struct Acceptor {
    config: Config,
    conn_id_seed: Key,
    conns: HashMap<ConnectionId<'static>, quiche::Connection>,
}

impl Acceptor {
    /// Create new acceptor.
    pub fn new(config: Config) -> io::Result<Self> {
        let rng = SystemRandom::new();

        let conn_id_seed = ring::hmac::Key::generate(ring::hmac::HMAC_SHA256, &rng)
            .map_err(|err| io::Error::new(io::ErrorKind::Other, format!("${err}")))?;

        Ok(Self {
            config,
            conn_id_seed,
            conns: Default::default(),
        })
    }

    fn conn_handshake(
        &mut self,
        conn: &mut quiche::Connection,
        from: SocketAddr,
        to: SocketAddr,
        buf: &mut [u8],
    ) -> io::Result<BytesMut> {
        _ = conn
            .recv(buf, quiche::RecvInfo { from, to })
            .map_err(into_io_error)?;

        let mut buf = ReadBuf::with_capacity(MAX_DATAGRAM_SIZE);

        let write_size = match conn.send(buf.as_mut()) {
            Ok((len, _)) => len,
            Err(quiche::Error::Done) => 0,
            Err(err) => return Err(into_io_error(err)),
        };

        Ok(buf.into_bytes_mut(Some(write_size)))
    }

    pub fn conn_timeout(
        &mut self,
        conn_id: &ConnectionId<'static>,
    ) -> io::Result<(BytesMut, Option<Duration>)> {
        let conn = self.conns.get_mut(conn_id).ok_or(io::Error::new(
            io::ErrorKind::NotFound,
            "Connection not found or removed by time expired",
        ))?;

        // raise timeout event.
        conn.on_timeout();

        let mut buf = ReadBuf::with_capacity(MAX_DATAGRAM_SIZE);

        let write_size = match conn.send(buf.as_mut()) {
            Ok((len, _)) => len,
            Err(quiche::Error::Done) => 0,
            Err(err) => return Err(into_io_error(err)),
        };

        Ok((buf.into_bytes_mut(Some(write_size)), conn.timeout()))
    }

    /// Recv new data.
    pub fn recv(
        &mut self,
        from: SocketAddr,
        to: SocketAddr,
        buf: &mut [u8],
    ) -> io::Result<Accept<'_>> {
        let header =
            quiche::Header::from_slice(buf, quiche::MAX_CONN_ID_LEN).map_err(into_io_error)?;

        log::trace!("Recv package {:?}", header.ty);

        match header.ty {
            quiche::Type::Initial => {
                if let Some(mut conn) = self.conns.remove(&header.dcid) {
                    let token = header.token.as_ref().unwrap();

                    // generate new token and retry
                    if token.is_empty() {
                        return Err(io::Error::new(
                            io::ErrorKind::ConnectionRefused,
                            "Quic token is null",
                        ));
                    }

                    _ = self.validate_token(token, &from)?;

                    let send_bytes = self.conn_handshake(&mut conn, from, to, buf)?;

                    if conn.is_established() {
                        return Ok(Accept::Incoming(QuicInnerConn::new(conn)));
                    } else {
                        let timeout = conn.timeout();
                        let conn_id = conn.source_id().into_owned();

                        self.conns.insert(header.dcid, conn);
                        return Ok(Accept::HandlingWithTimeout {
                            conn_id,
                            send_buf: send_bytes,
                            timeout,
                        });
                    }
                } else {
                    let conn_id = header.dcid.clone().into_owned();

                    let send_bytes = self.client_hello(from, to, header, buf)?;

                    if let Some(conn) = self.conns.get_mut(&conn_id) {
                        return Ok(Accept::HandlingWithTimeout {
                            conn_id,
                            send_buf: send_bytes,
                            timeout: conn.timeout(),
                        });
                    } else {
                        return Ok(Accept::Handling(send_bytes));
                    }
                }
            }
            quiche::Type::Handshake => {
                let mut conn = self.conns.remove(&header.dcid).ok_or(io::Error::new(
                    io::ErrorKind::ConnectionRefused,
                    format!("Quic connection not found, decid={:?}", header.dcid),
                ))?;

                let send_bytes = self.conn_handshake(&mut conn, from, to, buf)?;

                if conn.is_established() {
                    return Ok(Accept::Incoming(QuicInnerConn::new(conn)));
                } else {
                    let timeout = conn.timeout();

                    let conn_id = conn.source_id().into_owned();

                    self.conns.insert(header.dcid, conn);
                    return Ok(Accept::HandlingWithTimeout {
                        conn_id,
                        send_buf: send_bytes,
                        timeout,
                    });
                }
            }
            _ => {
                return Ok(Accept::Bypass(header));
            }
        }
    }

    fn client_hello(
        &mut self,
        from: SocketAddr,
        to: SocketAddr,
        header: Header<'_>,
        buf: &mut [u8],
    ) -> io::Result<BytesMut> {
        if !quiche::version_is_supported(header.version) {
            return self.negotiation_version(header);
        }

        let token = header.token.as_ref().unwrap();

        // generate new token and retry
        if token.is_empty() {
            return self.retry(from, header);
        }

        // check token .
        let odcid = self.validate_token(token, &from)?;

        let scid: ConnectionId<'_> = header.dcid.clone();

        if quiche::MAX_CONN_ID_LEN != scid.len() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                format!("Check dcid length error, len={}", scid.len()),
            ));
        }

        let mut conn = quiche::accept(&scid, Some(&odcid), to, from, &mut self.config)
            .map_err(into_io_error)?;

        let bytes = self.conn_handshake(&mut conn, from, to, buf)?;

        log::trace!(
            "Create new incoming conn, scid={:?},dcid={:?},handshake={}",
            conn.source_id(),
            conn.destination_id(),
            bytes.len(),
        );

        self.conns.insert(scid.into_owned(), conn);

        Ok(bytes)
    }

    /// Generate retry package
    fn retry(&self, from: SocketAddr, header: Header<'_>) -> io::Result<BytesMut> {
        let new_token = self.mint_token(&header, &from);

        let mut buf = ReadBuf::with_capacity(MAX_DATAGRAM_SIZE);

        let conn_id = ring::hmac::sign(&self.conn_id_seed, &header.dcid);
        let conn_id = &conn_id.as_ref()[..quiche::MAX_CONN_ID_LEN];
        let conn_id = ConnectionId::from_vec(conn_id.to_vec());

        let len = quiche::retry(
            &header.scid,
            &header.dcid,
            &conn_id,
            &new_token,
            header.version,
            buf.as_mut(),
        )
        .map_err(into_io_error)?;

        Ok(buf.into_bytes_mut(Some(len)))
    }

    fn validate_token<'a>(
        &self,
        token: &'a [u8],
        src: &SocketAddr,
    ) -> io::Result<quiche::ConnectionId<'a>> {
        if token.len() < 6 {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                format!("Invalid token, token length < 6"),
            ));
        }

        if &token[..6] != b"quiche" {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                format!("Invalid token, not start with 'quiche'"),
            ));
        }

        let token = &token[6..];

        let addr = match src.ip() {
            std::net::IpAddr::V4(a) => a.octets().to_vec(),
            std::net::IpAddr::V6(a) => a.octets().to_vec(),
        };

        if token.len() < addr.len() || &token[..addr.len()] != addr.as_slice() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                format!("Invalid token, address mismatch"),
            ));
        }

        Ok(quiche::ConnectionId::from_ref(&token[addr.len()..]))
    }

    fn mint_token<'a>(&self, hdr: &quiche::Header<'a>, src: &SocketAddr) -> Vec<u8> {
        let mut token = Vec::new();

        token.extend_from_slice(b"quiche");

        let addr = match src.ip() {
            std::net::IpAddr::V4(a) => a.octets().to_vec(),
            std::net::IpAddr::V6(a) => a.octets().to_vec(),
        };

        token.extend_from_slice(&addr);
        token.extend_from_slice(&hdr.dcid);

        token
    }

    fn negotiation_version<'a>(&self, header: Header<'a>) -> io::Result<BytesMut> {
        let mut buf = ReadBuf::with_capacity(MAX_DATAGRAM_SIZE);

        let len = quiche::negotiate_version(&header.scid, &header.dcid, buf.as_mut())
            .map_err(into_io_error)?;

        Ok(buf.into_bytes_mut(Some(len)))
    }
}
