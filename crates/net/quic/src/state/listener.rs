use std::{
    collections::{HashMap, VecDeque},
    io,
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use dashmap::DashMap;
use hala_future::{
    batching::FutureBatcher,
    event_map::{self, EventMap},
};
use hala_io::{bytes::BytesMut, *};
use hala_sync::{AsyncLockable, AsyncSpinMutex};
use quiche::{ConnectionId, RecvInfo, SendInfo};
use ring::{hmac::Key, rand::SystemRandom};

use crate::{errors::into_io_error, Config};

use super::QuicConnState;

/// [`handshake`](QuicAcceptor::handshake) result.
pub enum QuicAcceptorHandshake {
    Unhandled(quiche::ConnectionId<'static>),
    Incoming {
        conn: quiche::Connection,
        ping_timeout: Duration,
        write_size: usize,
        read_size: usize,
        send_info: SendInfo,
    },

    Internal {
        write_size: usize,
        read_size: usize,
        send_info: SendInfo,
    },
}

/// Raw incoming connection acceptor for quic server.
pub struct QuicAcceptor {
    /// Quic connection config
    config: Config,
    /// connection id seed.
    conn_id_seed: Key,
    /// connections before establishing connection.
    pre_established_conns: HashMap<ConnectionId<'static>, quiche::Connection>,
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
            pre_established_conns: Default::default(),
        })
    }

    /// Try to process quic init/handshake protocol and returns [`QuicAcceptorHandshake`] result
    pub fn handshake<'a>(
        &mut self,
        buf: &'a mut [u8],
        write_size: usize,
        recv_info: RecvInfo,
    ) -> io::Result<QuicAcceptorHandshake> {
        let header = quiche::Header::from_slice(&mut buf[..write_size], quiche::MAX_CONN_ID_LEN)
            .map_err(into_io_error)?;

        // this is pre-establishing conn packet
        if let Some(mut conn) = self.pre_established_conns.remove(&header.dcid) {
            let write_size = conn
                .recv(&mut buf[..write_size], recv_info)
                .map_err(into_io_error)?;

            let (read_size, send_info) = match conn.send(buf) {
                Ok(r) => r,
                Err(quiche::Error::Done) => (
                    0,
                    SendInfo {
                        from: recv_info.to,
                        to: recv_info.from,
                        at: Instant::now(),
                    },
                ),
                Err(err) => {
                    return Err(into_io_error(err));
                }
            };

            if conn.is_established() {
                return Ok(QuicAcceptorHandshake::Incoming {
                    conn,
                    ping_timeout: self.config.send_ping_interval,
                    write_size,
                    read_size,
                    send_info,
                });
            } else {
                self.pre_established_conns
                    .insert(header.dcid.clone().into_owned(), conn);

                return Ok(QuicAcceptorHandshake::Internal {
                    write_size,
                    read_size,
                    send_info,
                });
            }
        }

        if header.ty == quiche::Type::Initial {
            return self.client_hello(&header, buf, write_size, recv_info);
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
        write_size: usize,
        recv_info: RecvInfo,
    ) -> io::Result<QuicAcceptorHandshake> {
        if !quiche::version_is_supported(header.version) {
            return self.negotiation_version(header, recv_info, buf, write_size);
        }

        let token = header.token.as_ref().unwrap();

        // generate new token and retry
        if token.is_empty() {
            return self.retry(header, recv_info, buf, write_size);
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

        let write_size = conn
            .recv(&mut buf[..write_size], recv_info)
            .map_err(into_io_error)?;

        log::trace!(
            "Create new incoming conn, scid={:?}, dcid={:?}, write_size={}",
            conn.source_id(),
            conn.destination_id(),
            write_size,
        );

        let (read_size, send_info) = match conn.send(buf) {
            Ok(r) => r,
            Err(quiche::Error::Done) => (
                0,
                SendInfo {
                    from: recv_info.to,
                    to: recv_info.from,
                    at: Instant::now(),
                },
            ),
            Err(err) => {
                return Err(into_io_error(err));
            }
        };

        if conn.is_established() {
            return Ok(QuicAcceptorHandshake::Incoming {
                conn,
                write_size,
                read_size,
                ping_timeout: self.config.send_ping_interval,
                send_info,
            });
        } else {
            self.pre_established_conns
                .insert(scid.clone().into_owned(), conn);

            return Ok(QuicAcceptorHandshake::Internal {
                write_size,
                read_size,
                send_info,
            });
        }
    }

    /// Generate retry package
    fn retry<'a>(
        &mut self,
        header: &quiche::Header<'a>,
        recv_info: RecvInfo,
        buf: &mut [u8],
        write_size: usize,
    ) -> io::Result<QuicAcceptorHandshake> {
        let token = self.mint_token(&header, &recv_info.from);

        let new_scid = ring::hmac::sign(&self.conn_id_seed, &header.dcid);
        let new_scid = &new_scid.as_ref()[..quiche::MAX_CONN_ID_LEN];
        let new_scid = quiche::ConnectionId::from_vec(new_scid.to_vec());

        let scid = header.scid.clone().into_owned();
        let dcid: ConnectionId<'_> = header.dcid.clone().into_owned();
        let version = header.version;

        let read_size =
            quiche::retry(&scid, &dcid, &new_scid, &token, version, buf).map_err(into_io_error)?;

        Ok(QuicAcceptorHandshake::Internal {
            write_size,
            read_size,
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
        buf: &mut [u8],
        write_size: usize,
    ) -> io::Result<QuicAcceptorHandshake> {
        let scid = header.scid.clone().into_owned();
        let dcid = header.dcid.clone().into_owned();

        let read_size = quiche::negotiate_version(&scid, &dcid, buf).map_err(into_io_error)?;

        Ok(QuicAcceptorHandshake::Internal {
            write_size,
            read_size,
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

/// Result returns by [`write`](QuicListenerState::write) function.
#[must_use = "Must handle internal data forward"]
pub enum QuicListenerWriteResult {
    WriteSize(usize),
    Internal {
        write_size: usize,
        read_size: usize,
        send_info: SendInfo,
    },
    Incoming {
        conn: QuicConnState,
        write_size: usize,
        read_size: usize,
        send_info: SendInfo,
    },
}

enum QuicListnerConnRead {
    Err(QuicConnState, io::Error),
    Ok(QuicConnState, BytesMut, SendInfo),
}

/// The state machine for quic server listener.
#[derive(Clone)]
pub struct QuicListenerState {
    closed: Arc<AtomicBool>,
    /// the acceptor for incoming connections.
    acceptor: Arc<AsyncSpinMutex<QuicAcceptor>>,
    /// The set of activing [`QuicConnState`]s.
    conns: Arc<DashMap<quiche::ConnectionId<'static>, QuicConnState>>,
    /// The queue for new incoming connections.
    incoming: Arc<AsyncSpinMutex<Option<VecDeque<QuicConnState>>>>,
    /// event notify center.
    mediator: Arc<EventMap<QuicListenerStateEvent>>,
    /// the batch processor for reading data from connections .
    conns_read: Arc<FutureBatcher<QuicListnerConnRead>>,
    /// quic mtu size.
    max_datagram_size: usize,
}

impl QuicListenerState {
    /// Use [`config`](Config) to create new [`QuicListenerState`]
    pub fn new(config: Config) -> io::Result<Self> {
        Ok(Self {
            max_datagram_size: config.max_datagram_size,
            acceptor: Arc::new(AsyncSpinMutex::new(QuicAcceptor::new(config)?)),
            conns: Default::default(),
            incoming: Arc::new(AsyncSpinMutex::new(Some(Default::default()))),
            mediator: Default::default(),
            conns_read: Default::default(),
            closed: Default::default(),
        })
    }

    /// Processes QUIC packets received from the peer.
    pub async fn write(
        &self,
        buf: &mut [u8],
        write_size: usize,
        recv_info: RecvInfo,
    ) -> io::Result<QuicListenerWriteResult> {
        self.handle_listener_statuts()?;

        let handshake = {
            let mut acceptor = self.acceptor.lock().await;

            acceptor.handshake(buf, write_size, recv_info)?
        };

        match handshake {
            QuicAcceptorHandshake::Unhandled(dcid) => {
                let conn = self.conns.get(&dcid).map(|conn| conn.clone());

                if let Some(conn) = conn {
                    log::trace!(
                        "Listener write data to conn, scid={:?}, dcid={:?}",
                        conn.scid,
                        conn.dcid
                    );

                    if let Err(err) = conn.write(&mut buf[..write_size], recv_info).await {
                        log::error!("{:?}, write data error, {}", conn, err);

                        self.conns.remove(&dcid);
                    }

                    return Ok(QuicListenerWriteResult::WriteSize(write_size));
                }

                log::error!("Quic conn not found, scid={:?}", dcid);

                return Ok(QuicListenerWriteResult::WriteSize(write_size));
            }
            QuicAcceptorHandshake::Incoming {
                conn,
                ping_timeout,
                write_size,
                read_size,
                send_info,
            } => {
                log::trace!(
                    "accept incoming conn, scid={:?}, dcid={:?}",
                    conn.source_id(),
                    conn.destination_id()
                );

                let scid = conn.source_id().clone().into_owned();

                let conn = QuicConnState::new(conn, ping_timeout, 5);

                self.conns.insert(scid.clone(), conn.clone());

                self.incoming
                    .lock()
                    .await
                    .as_mut()
                    .ok_or(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "quic listener state closed.",
                    ))?
                    .push_back(conn.clone());

                self.mediator
                    .notify_one(QuicListenerStateEvent::Incoming, event_map::Reason::On);

                self.batch_read(conn.clone());

                return Ok(QuicListenerWriteResult::Incoming {
                    conn,
                    write_size,
                    read_size,
                    send_info,
                });
            }
            QuicAcceptorHandshake::Internal {
                write_size,
                read_size,
                send_info,
            } => {
                return Ok(QuicListenerWriteResult::Internal {
                    write_size,
                    read_size,
                    send_info,
                });
            }
        }
    }

    fn handle_listener_statuts(&self) -> io::Result<()> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "Listener closed"));
        }

        Ok(())
    }

    /// Close this listener and drop the incoming queue.
    pub async fn close(&self) {
        if self
            .closed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            // try wakeup read future.
            self.conns_read.close();
            return;
        }

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

    fn batch_read(&self, conn: QuicConnState) {
        let max_datagram_size = self.max_datagram_size;

        // push new task into batch poller.
        self.conns_read.push(async move {
            let mut buf = ReadBuf::with_capacity(max_datagram_size);

            let read = match conn.read(buf.chunk_mut()).await {
                Ok((read_size, send_info)) => {
                    QuicListnerConnRead::Ok(conn, buf.into_bytes_mut(Some(read_size)), send_info)
                }
                Err(err) => QuicListnerConnRead::Err(conn, err),
            };

            read
        });
    }

    pub async fn read(&self) -> io::Result<(BytesMut, SendInfo)> {
        self.handle_listener_statuts()?;

        loop {
            match self.conns_read.wait().await {
                Some(QuicListnerConnRead::Err(conn, err)) => {
                    log::trace!(
                        "QuicListener, read data from conn error. scid={:?}, dcid={:?}, err={}",
                        conn.scid,
                        conn.dcid,
                        err
                    );

                    self.handle_listener_statuts()?;

                    // remove broken conn
                    self.conns.remove(&conn.scid);

                    continue;
                }
                Some(QuicListnerConnRead::Ok(conn, buf, send_info)) => {
                    self.handle_listener_statuts()?;

                    // push conn into batch poller again.
                    self.batch_read(conn);

                    return Ok((buf, send_info));
                }
                None => {
                    self.handle_listener_statuts()?;
                }
            }
        }
    }
}
