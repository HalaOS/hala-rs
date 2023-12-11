use std::{collections::HashMap, io, net::SocketAddr};

use quiche::{ConnectionId, Header};
use ring::{hmac::Key, rand::SystemRandom};

use crate::errors::into_io_error;

use super::{inner_conn::QuicInnerConn, Config};

pub struct ServerHello {
    pub read_size: usize,
    pub write_size: usize,
    pub conn: Option<QuicInnerConn>,
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

    fn conn_recv_send(
        conn: &mut quiche::Connection,
        in_buf: &mut [u8],
        out_buf: &mut [u8],
        from: SocketAddr,
        to: SocketAddr,
    ) -> io::Result<(usize, usize)> {
        let read_size = conn
            .recv(in_buf, quiche::RecvInfo { from, to })
            .map_err(into_io_error)?;

        let write_size = match conn.send(out_buf) {
            Ok((len, _)) => len,
            Err(quiche::Error::Done) => 0,
            Err(err) => return Err(into_io_error(err)),
        };

        Ok((read_size, write_size))
    }

    pub fn client_hello(
        &mut self,
        header: Header<'_>,
        in_buf: &mut [u8],
        out_buf: &mut [u8],
        from: SocketAddr,
        to: SocketAddr,
    ) -> io::Result<ServerHello> {
        let conn_id = ring::hmac::sign(&self.conn_id_seed, &header.dcid);
        let conn_id = &conn_id.as_ref()[..quiche::MAX_CONN_ID_LEN];
        let conn_id = ConnectionId::from_vec(conn_id.to_vec());

        let decid = header.dcid.clone().into_owned();

        let conn = if let Some(conn) = self.conns.remove(&decid) {
            Some(conn)
        } else {
            self.conns.remove(&conn_id)
        };

        if let Some(mut conn) = conn {
            let (read_size, write_size) =
                Self::conn_recv_send(&mut conn, in_buf, out_buf, from, to)?;

            if conn.is_established() {
                return Ok(ServerHello {
                    write_size,
                    read_size,
                    conn: Some(QuicInnerConn::new(conn)),
                });
            } else {
                self.conns.insert(conn.source_id().into_owned(), conn);

                return Ok(ServerHello {
                    write_size,
                    read_size,
                    conn: None,
                });
            }
        }

        if header.ty != quiche::Type::Initial {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                format!("Expect init package, but got {:?}", header.ty),
            ));
        }

        if !quiche::version_is_supported(header.version) {
            return self
                .negotiation_version(header, out_buf)
                .map(|len| ServerHello {
                    write_size: len,
                    read_size: in_buf.len(),
                    conn: None,
                });
        }

        let token = header.token.as_ref().unwrap();

        // Do stateless retry if the client didn't send a token.
        if token.is_empty() {
            log::trace!(
                "Acceptor retry scid={:?},dcid={:?},new_scid={:?}",
                header.scid,
                header.dcid,
                conn_id
            );

            let new_token = self.mint_token(&header, &from);

            return quiche::retry(
                &header.scid,
                &header.dcid,
                &conn_id,
                &new_token,
                header.version,
                out_buf,
            )
            .map_err(into_io_error)
            .map(|len| ServerHello {
                write_size: len,
                read_size: in_buf.len(),
                conn: None,
            });
        }

        let odcid = self.validate_token(token, &from)?;

        if conn_id.len() != header.dcid.len() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                format!(
                    "Invalid destination connection ID, expect len = {}",
                    conn_id.len()
                ),
            ));
        }

        let scid: ConnectionId<'_> = header.dcid.clone();

        let mut conn =
            quiche::accept(&scid, Some(&odcid), to, from, &mut self.config).map_err(|err| {
                io::Error::new(
                    io::ErrorKind::Interrupted,
                    format!("Accept connection failed,{:?}", err),
                )
            })?;

        let (read_size, write_size) = Self::conn_recv_send(&mut conn, in_buf, out_buf, from, to)?;

        if conn.is_established() {
            return Ok(ServerHello {
                write_size,
                read_size,
                conn: Some(QuicInnerConn::new(conn)),
            });
        } else {
            self.conns.insert(scid.into_owned(), conn);

            return Ok(ServerHello {
                write_size,
                read_size,
                conn: None,
            });
        }
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

    fn negotiation_version<'a>(&mut self, header: Header<'a>, buf: &mut [u8]) -> io::Result<usize> {
        quiche::negotiate_version(&header.scid, &header.dcid, buf).map_err(into_io_error)
    }
}
