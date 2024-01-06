use std::{
    collections::VecDeque,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use dashmap::DashMap;
use event_map::{
    locks::{AsyncLockable, AsyncSpinMutex},
    EventMap,
};
use futures::io;
use quiche::{RecvInfo, SendInfo};
use ring::{hmac::Key, rand::SystemRandom};

use crate::{errors::into_io_error, Config};

use super::QuicConnState;

/// [`handshake`](Acceptor::handshake) result.
pub enum QuicAcceptorHandshake {
    Unhandled(quiche::ConnectionId<'static>),
    Incoming {
        conn: quiche::Connection,
        ping_timeout: Duration,
        write_size: usize,
    },

    Retry {
        scid: quiche::ConnectionId<'static>,
        dcid: quiche::ConnectionId<'static>,
        new_scid: quiche::ConnectionId<'static>,
        token: Vec<u8>,
        version: u32,
        send_info: SendInfo,
    },

    NegotiationVersion {
        scid: quiche::ConnectionId<'static>,
        dcid: quiche::ConnectionId<'static>,
        send_info: SendInfo,
    },
}

impl QuicAcceptorHandshake {
    /// Try writes handshake result to be sent to the peer.
    pub fn send(&self, buf: &mut [u8]) -> io::Result<Option<(usize, SendInfo)>> {
        match self {
            QuicAcceptorHandshake::Unhandled(_) => Ok(None),
            QuicAcceptorHandshake::Incoming {
                conn: _,
                write_size: _,
                ping_timeout: _,
            } => Ok(None),
            QuicAcceptorHandshake::Retry {
                scid,
                dcid,
                new_scid,
                token,
                version,
                send_info,
            } => {
                let send_size = quiche::retry(scid, dcid, new_scid, token, *version, buf)
                    .map_err(into_io_error)?;

                return Ok(Some((send_size, *send_info)));
            }
            QuicAcceptorHandshake::NegotiationVersion {
                scid,
                dcid,
                send_info,
            } => {
                let send_size =
                    quiche::negotiate_version(scid, dcid, buf).map_err(into_io_error)?;

                return Ok(Some((send_size, *send_info)));
            }
        }
    }
}

/// Raw incoming connection acceptor for quic server.
pub struct QuicAcceptor {
    /// Quic connection config
    config: Config,
    /// connection id seed.
    conn_id_seed: Key,
}

impl QuicAcceptor {
    /// Create new acceptor with the given quic connection [`config`](Config)
    pub fn new(config: Config) -> io::Result<Self> {
        let rng = SystemRandom::new();

        let conn_id_seed = ring::hmac::Key::generate(ring::hmac::HMAC_SHA256, &rng)
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;

        Ok(Self {
            config,
            conn_id_seed,
        })
    }

    /// Try to process quic init/handshake protocol and returns [`Handshake`] result
    pub fn handshake<'a>(
        &mut self,
        buf: &'a mut [u8],
        recv_info: RecvInfo,
    ) -> io::Result<QuicAcceptorHandshake> {
        let header =
            quiche::Header::from_slice(buf, quiche::MAX_CONN_ID_LEN).map_err(into_io_error)?;

        if header.ty == quiche::Type::Initial {
            return self.client_hello(&header, buf, recv_info);
        } else {
            return Ok(QuicAcceptorHandshake::Unhandled(
                header.dcid.clone().into_owned(),
            ));
        }
    }

    fn client_hello<'a>(
        &mut self,
        header: &quiche::Header<'a>,
        buf: &'a mut [u8],
        recv_info: RecvInfo,
    ) -> io::Result<QuicAcceptorHandshake> {
        if !quiche::version_is_supported(header.version) {
            return self.negotiation_version(header, recv_info);
        }

        let token = header.token.as_ref().unwrap();

        // generate new token and retry
        if token.is_empty() {
            return self.retry(header, recv_info);
        }

        // check token .
        let odcid = Self::validate_token(token, &recv_info.from)?;

        let scid: quiche::ConnectionId<'_> = header.dcid.clone();

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
            "Create new incoming conn, scid={:?}, dcid={:?}, read_size={}",
            conn.source_id(),
            conn.destination_id(),
            read_size,
        );

        return Ok(QuicAcceptorHandshake::Incoming {
            conn,
            write_size: read_size,
            ping_timeout: self.config.ping_timeout,
        });
    }

    /// Generate retry package
    fn retry<'a>(
        &mut self,
        header: &quiche::Header<'a>,
        recv_info: RecvInfo,
    ) -> io::Result<QuicAcceptorHandshake> {
        let token = self.mint_token(&header, &recv_info.from);

        let new_scid = ring::hmac::sign(&self.conn_id_seed, &header.dcid);
        let new_scid = &new_scid.as_ref()[..quiche::MAX_CONN_ID_LEN];
        let new_scid = quiche::ConnectionId::from_vec(new_scid.to_vec());

        Ok(QuicAcceptorHandshake::Retry {
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
        })
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

    fn negotiation_version<'a>(
        &mut self,
        header: &quiche::Header<'a>,
        recv_info: RecvInfo,
    ) -> io::Result<QuicAcceptorHandshake> {
        Ok(QuicAcceptorHandshake::NegotiationVersion {
            scid: header.scid.clone().into_owned(),
            dcid: header.dcid.clone().into_owned(),
            send_info: SendInfo {
                from: recv_info.to,
                to: recv_info.from,
                at: Instant::now(),
            },
        })
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
enum QuicListenerStateEvent {
    Incoming,
}

/// The state machine for quic server listener.
pub struct QuicListenerState {
    /// the acceptor for incoming connections.
    acceptor: Arc<AsyncSpinMutex<QuicAcceptor>>,
    /// The set of activing [`QuicConnState`]s.
    conns: Arc<DashMap<quiche::ConnectionId<'static>, QuicConnState>>,
    /// The queue for new incoming connections.
    incoming: Arc<AsyncSpinMutex<Option<VecDeque<QuicConnState>>>>,
    /// event notify center.
    mediator: Arc<EventMap<QuicListenerStateEvent>>,
}

/// Result returns by [`QuicListenerState::recv`](QuicListenerState::recv)
pub enum QuicListenerStateRecv {
    WriteSize(usize),
    Handshake(usize, QuicAcceptorHandshake),
}

impl QuicListenerState {
    /// Use [`config`](Config) to create new [`QuicListenerState`]
    pub fn new(config: Config) -> io::Result<Self> {
        Ok(Self {
            acceptor: Arc::new(AsyncSpinMutex::new(QuicAcceptor::new(config)?)),
            conns: Default::default(),
            incoming: Arc::new(AsyncSpinMutex::new(Some(Default::default()))),
            mediator: Default::default(),
        })
    }

    /// Processes QUIC packets received from the peer.
    pub async fn write(
        &self,
        buf: &mut [u8],
        recv_info: RecvInfo,
    ) -> io::Result<QuicListenerStateRecv> {
        let handshake = {
            let mut acceptor = self.acceptor.lock().await;

            acceptor.handshake(buf, recv_info)?
        };

        match handshake {
            QuicAcceptorHandshake::Unhandled(dcid) => {
                let conn = self.conns.get_mut(&dcid).map(|conn| conn.clone());

                if let Some(conn) = conn {
                    return conn
                        .write(buf, recv_info)
                        .await
                        .map(|write_size| QuicListenerStateRecv::WriteSize(write_size));
                } else {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("Quic conn not found, scid={:?}", dcid),
                    ));
                }
            }
            QuicAcceptorHandshake::Incoming {
                conn,
                ping_timeout,
                write_size,
            } => {
                log::info!(
                    "accept incoming conn, scid={:?}, dcid={:?}",
                    conn.source_id(),
                    conn.destination_id()
                );

                let scid = conn.source_id().clone().into_owned();

                let conn = QuicConnState::new(conn, ping_timeout, 5);

                self.conns.insert(scid, conn.clone());

                self.incoming
                    .lock()
                    .await
                    .as_mut()
                    .ok_or(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "quic listener state closed.",
                    ))?
                    .push_back(conn);

                self.mediator
                    .notify_one(QuicListenerStateEvent::Incoming, event_map::Reason::On);

                return Ok(QuicListenerStateRecv::WriteSize(write_size));
            }
            handshake => {
                return Ok(QuicListenerStateRecv::Handshake(buf.len(), handshake));
            }
        }
    }

    /// Close this listener.
    pub async fn close(&self) {
        let mut incoming = self.incoming.lock().await;

        *incoming = None;

        self.mediator
            .notify_one(QuicListenerStateEvent::Incoming, event_map::Reason::On);
    }

    /// Accept one incoming connection, or returns `None` if this listener had been closed.
    pub async fn accept(&self) -> Option<QuicConnState> {
        loop {
            let mut incoming = self.incoming.lock().await;

            if let Some(incoming) = incoming.as_mut() {
                if let Some(conn) = incoming.pop_front() {
                    return Some(conn);
                }
            } else {
                return None;
            }

            // Safety: the close function uses `event_map::Reason::On` to notify incoming listener
            self.mediator
                .wait(QuicListenerStateEvent::Incoming, incoming)
                .await
                .expect("Please always use `event_map::Reason::On` to notify me!!");
        }
    }
}
