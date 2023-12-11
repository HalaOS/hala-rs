use std::{io, net::SocketAddr};

use quiche::{ConnectionId, Header};
use ring::hmac::Key;

use crate::errors::into_io_error;

use super::{inner_conn::QuicInnerConn, MAX_DATAGRAM_SIZE};

/// Quic server connection acceptor
pub struct Acceptor {
    config: quiche::Config,
    conn_id_seed: Key,
}

impl Acceptor {
    pub fn handle_client_hello<'a>(
        &mut self,
        header: Header<'a>,
        buf: &mut [u8],
        from: SocketAddr,
        to: SocketAddr,
    ) -> io::Result<(usize, Option<QuicInnerConn>)> {
        if header.ty != quiche::Type::Initial {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                format!("Expect init package, but got {:?}", header.ty),
            ));
        }

        if !quiche::version_is_supported(header.version) {
            return self.negotiation_version(header, buf).map(|len| (len, None));
        }

        let token = header.token.as_ref().unwrap();

        let mut buf = [0; MAX_DATAGRAM_SIZE];

        let conn_id = ring::hmac::sign(&self.conn_id_seed, &header.dcid);
        let conn_id = &conn_id.as_ref()[..quiche::MAX_CONN_ID_LEN];
        let conn_id = ConnectionId::from_vec(conn_id.to_vec());

        // Do stateless retry if the client didn't send a token.
        if token.is_empty() {
            let new_token = self.mint_token(&header, &from);

            let len = quiche::retry(
                &header.scid,
                &header.dcid,
                &conn_id,
                &new_token,
                header.version,
                &mut buf,
            )
            .unwrap();

            return Ok((len, None));
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

        log::debug!("New connection: dcid={:?} scid={:?}", header.dcid, scid);

        let conn =
            quiche::accept(&scid, Some(&odcid), to, from, &mut self.config).map_err(|err| {
                io::Error::new(
                    io::ErrorKind::Interrupted,
                    format!("Accept connection failed,{:?}", err),
                )
            })?;

        return Ok((0, Some(QuicInnerConn::new(conn))));
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
