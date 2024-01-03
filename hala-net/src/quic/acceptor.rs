use std::{
    collections::{HashMap, VecDeque},
    io,
    net::SocketAddr,
    time::Instant,
};

use quiche::{ConnectionId, Header, RecvInfo, SendInfo};
use rand::seq::IteratorRandom;
use ring::{hmac::Key, rand::SystemRandom};

use crate::{
    errors::into_io_error,
    quic::{Config, QuicConn},
};

use super::conn_state::QuicConnState;

enum InitAck {
    NegotiationVersion {
        scid: ConnectionId<'static>,
        dcid: ConnectionId<'static>,
        send_info: SendInfo,
    },
    Retry {
        scid: ConnectionId<'static>,
        dcid: ConnectionId<'static>,
        new_scid: ConnectionId<'static>,
        token: Vec<u8>,
        version: u32,
        send_info: SendInfo,
    },
}

/// Quic server connection acceptor
pub(super) struct QuicAcceptor {
    config: Config,
    conn_id_seed: Key,
    conns: HashMap<ConnectionId<'static>, quiche::Connection>,
    init_acks: VecDeque<InitAck>,
}

impl QuicAcceptor {
    /// Create new acceptor.
    pub fn new(config: Config) -> io::Result<Self> {
        let rng = SystemRandom::new();

        let conn_id_seed = ring::hmac::Key::generate(ring::hmac::HMAC_SHA256, &rng)
            .map_err(|err| io::Error::new(io::ErrorKind::Other, format!("${err}")))?;

        Ok(Self {
            config,
            conn_id_seed,
            conns: Default::default(),
            init_acks: Default::default(),
        })
    }

    pub fn pop_established(&mut self) -> Vec<(ConnectionId<'static>, QuicConn)> {
        let ids = self
            .conns
            .values()
            .filter(|conn| conn.is_established())
            .map(|conn| conn.source_id().clone().into_owned())
            .collect::<Vec<_>>();

        let mut conns = vec![];

        for id in ids {
            log::info!("established conn={:?}", id);

            let state = QuicConn::new(QuicConnState::new(self.conns.remove(&id).unwrap(), 5));
            conns.push((id, state));
        }

        conns
    }

    pub fn send(&mut self, buf: &mut [u8]) -> io::Result<(usize, SendInfo)> {
        // first handle init acks
        if let Some(ack) = self.init_acks.pop_front() {
            match ack {
                InitAck::NegotiationVersion {
                    scid,
                    dcid,
                    send_info,
                } => {
                    let send_size =
                        quiche::negotiate_version(&scid, &dcid, buf).map_err(into_io_error)?;

                    return Ok((send_size, send_info));
                }
                InitAck::Retry {
                    scid,
                    dcid,
                    new_scid,
                    token,
                    version,
                    send_info,
                } => {
                    let send_size = quiche::retry(&scid, &dcid, &new_scid, &token, version, buf)
                        .map_err(into_io_error)?;

                    return Ok((send_size, send_info));
                }
            }
        }

        // secondary, handle conn send

        let len = self.conns.len();

        let conns = self
            .conns
            .values_mut()
            .choose_multiple(&mut rand::thread_rng(), len);

        for conn in conns {
            match conn.send(buf) {
                Ok((send_size, send_info)) => {
                    return Ok((send_size, send_info));
                }
                Err(quiche::Error::Done) => {}
                Err(_) => todo!(),
            }
        }

        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "No more data to send",
        ));
    }

    /// Recv new from remote.
    pub fn recv<'a>(
        &mut self,
        buf: &'a mut [u8],
        recv_info: RecvInfo,
    ) -> io::Result<(usize, Header<'a>)> {
        let header =
            quiche::Header::from_slice(buf, quiche::MAX_CONN_ID_LEN).map_err(into_io_error)?;

        log::trace!("Recv package {:?}", header.ty);

        match header.ty {
            quiche::Type::Initial => {
                if let Some(conn) = self.conns.get_mut(&header.dcid) {
                    let token = header.token.as_ref().unwrap();

                    // generate new token and retry
                    if token.is_empty() {
                        return Err(io::Error::new(
                            io::ErrorKind::ConnectionRefused,
                            "Quic token is null",
                        ));
                    }

                    _ = Self::validate_token(token, &recv_info.from)?;

                    let read_size = conn.recv(buf, recv_info).map_err(into_io_error)?;

                    return Ok((read_size, header));
                } else {
                    let len = self.client_hello(&header, buf, recv_info)?;

                    return Ok((len, header));
                }
            }
            quiche::Type::Handshake => {
                if let Some(conn) = self.conns.get_mut(&header.dcid) {
                    return conn
                        .recv(buf, recv_info)
                        .map_err(into_io_error)
                        .map(|len| (len, header));
                } else {
                    return Ok((0, header));
                }
            }
            _ => {
                return Ok((0, header));
            }
        }
    }

    fn client_hello(
        &mut self,
        header: &Header<'_>,
        buf: &mut [u8],
        recv_info: RecvInfo,
    ) -> io::Result<usize> {
        if !quiche::version_is_supported(header.version) {
            self.negotiation_version(header, recv_info)?;

            return Ok(buf.len());
        }

        let token = header.token.as_ref().unwrap();

        // generate new token and retry
        if token.is_empty() {
            self.retry(header, recv_info)?;

            return Ok(buf.len());
        }

        // check token .
        let odcid = Self::validate_token(token, &recv_info.from)?;

        let scid: ConnectionId<'_> = header.dcid.clone();

        if quiche::MAX_CONN_ID_LEN != scid.len() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                format!("Check dcid length error, len={}", scid.len()),
            ));
        }

        let mut conn = quiche::accept(
            &scid,
            Some(&odcid),
            recv_info.to,
            recv_info.from,
            &mut self.config,
        )
        .map_err(into_io_error)?;

        let read_size = conn.recv(buf, recv_info).map_err(into_io_error)?;

        log::trace!(
            "Create new incoming conn, scid={:?},dcid={:?},handshake={}",
            conn.source_id(),
            conn.destination_id(),
            read_size,
        );

        self.conns.insert(scid.into_owned(), conn);

        Ok(read_size)
    }

    /// Generate retry package
    fn retry(&mut self, header: &Header<'_>, recv_info: RecvInfo) -> io::Result<()> {
        let token = self.mint_token(&header, &recv_info.from);

        let new_scid = ring::hmac::sign(&self.conn_id_seed, &header.dcid);
        let new_scid = &new_scid.as_ref()[..quiche::MAX_CONN_ID_LEN];
        let new_scid = ConnectionId::from_vec(new_scid.to_vec());

        self.init_acks.push_back(InitAck::Retry {
            scid: header.scid.clone().into_owned(),
            dcid: header.dcid.clone().into_owned(),
            new_scid,
            token,
            version: header.version,
            send_info: SendInfo {
                from: recv_info.to,
                to: recv_info.from,
                at: Instant::now(),
            },
        });

        Ok(())
    }

    fn validate_token<'a>(
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

    fn negotiation_version(&mut self, header: &Header<'_>, recv_info: RecvInfo) -> io::Result<()> {
        self.init_acks.push_back(InitAck::NegotiationVersion {
            scid: header.scid.clone().into_owned(),
            dcid: header.dcid.clone().into_owned(),
            send_info: SendInfo {
                from: recv_info.to,
                to: recv_info.from,
                at: Instant::now(),
            },
        });

        Ok(())
    }
}
