use std::{
    collections::HashMap,
    fmt::Debug,
    io,
    net::SocketAddr,
    time::{Duration, Instant},
};

use futures::{
    channel::mpsc::{Receiver, Sender},
    SinkExt, StreamExt,
};
use hala_future::executor::future_spawn;
use hala_io::bytes::{BufMut, Bytes, BytesMut};
use quiche::{ConnectionId, RecvInfo, SendInfo};
use ring::{hmac::Key, rand::SystemRandom};

use crate::{cs::QuicConn, errors::into_io_error, Config};

use super::QuicConnCmd;

/// [`handshake`](Acceptor::handshake) result.
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

    /// Try to process quic init/handshake protocol and returns [`Handshake`] result
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

pub struct QuicListenerState {
    /// label for log trace.
    trace_label: String,
    /// quic mtu size.
    max_datagram_size: usize,
    /// The max cache len of quic connection command queue.
    max_conn_state_cache_len: usize,
    /// Quic acceptor state machine.
    acceptor: QuicAcceptor,
    /// Register incoming conns.
    conns: HashMap<ConnectionId<'static>, Sender<QuicConnCmd>>,
    /// The State machine data sender
    listener_data_sender: Sender<(Bytes, SendInfo)>,
    /// The State machine data receiver.
    listener_data_receiver: Receiver<(BytesMut, RecvInfo)>,
    /// incoming conn sender.
    incoming_conn_sender: Sender<QuicConn>,
}

impl Debug for QuicListenerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.trace_label)
    }
}

impl QuicListenerState {
    /// Create new Quic listener state machine using provided [`Config`]
    pub fn new(
        trace_label: String,
        config: Config,
        listener_data_sender: Sender<(Bytes, SendInfo)>,
        listener_data_receiver: Receiver<(BytesMut, RecvInfo)>,
        incoming_conn_sender: Sender<QuicConn>,
    ) -> io::Result<Self> {
        Ok(Self {
            trace_label,
            max_conn_state_cache_len: config.max_conn_state_cache_len,
            max_datagram_size: config.max_datagram_size,
            acceptor: QuicAcceptor::new(config)?,
            conns: Default::default(),
            listener_data_receiver,
            listener_data_sender,
            incoming_conn_sender,
        })
    }

    /// Consume self and run state machine
    pub fn start(mut self) {
        future_spawn(async move {
            log::trace!("{:?}, started", self);

            match self.event_loop().await {
                Ok(_) => {
                    log::info!("{:?}, stopped", self);
                }
                Err(err) => {
                    log::info!("{:?}, stopped due to error, {}", self, err);
                }
            }
        });
    }

    async fn event_loop(&mut self) -> io::Result<()> {
        let mut buf = vec![0u8; self.max_datagram_size];

        while let Some((recv_buf, recv_info)) = self.listener_data_receiver.next().await {
            let write_size = recv_buf.len();
            assert!(
                self.max_datagram_size >= write_size,
                "Listener received buf > max_datagram_size"
            );

            buf[..write_size].copy_from_slice(&recv_buf);

            let handshake = match self.acceptor.handshake(&mut buf, write_size, recv_info) {
                Ok(handshake) => handshake,
                Err(err) => {
                    log::error!("{:?}, recv invalid data({}), {}", self, write_size, err);
                    continue;
                }
            };

            match handshake {
                QuicAcceptorHandshake::Unhandled(dcid) => {
                    if let Some(mut conn) = self.conns.remove(&dcid) {
                        if conn
                            .send(QuicConnCmd::Recv {
                                buf: recv_buf,
                                recv_info,
                            })
                            .await
                            .is_ok()
                        {
                            self.conns.insert(dcid, conn);
                        } else {
                            log::info!("{:?}, connection removed, scid={:?}", self, dcid);
                        }
                    } else {
                        log::error!(
                            "{:?}, conn not found, scid={:?}, recv_info={:?}",
                            self,
                            dcid,
                            recv_info
                        );
                    }
                }
                QuicAcceptorHandshake::Incoming {
                    conn,
                    ping_timeout,
                    write_size: _,
                    read_size,
                    send_info,
                } => {
                    log::info!(
                        "accept incoming conn, scid={:?}, dcid={:?}",
                        conn.source_id(),
                        conn.destination_id()
                    );

                    let (conn, state) = QuicConn::make_server_conn(
                        self.listener_data_sender.clone(),
                        conn,
                        ping_timeout,
                        self.max_conn_state_cache_len,
                    );

                    state.start();

                    self.conns
                        .insert(conn.source_id().clone(), conn.event_sender());

                    if read_size > 0 {
                        let mut send_buf = BytesMut::new();

                        send_buf.put(&buf[..read_size]);

                        self.listener_data_sender
                            .send((send_buf.into(), send_info))
                            .await
                            .map_err(into_io_error)?;
                    }

                    // Send new incoming quic connection instance.
                    self.incoming_conn_sender
                        .send(conn)
                        .await
                        .map_err(into_io_error)?;
                }
                QuicAcceptorHandshake::Internal {
                    write_size: _,
                    read_size,
                    send_info,
                } => {
                    if read_size > 0 {
                        let mut send_buf = BytesMut::new();

                        send_buf.put(&buf[..read_size]);

                        self.listener_data_sender
                            .send((send_buf.into(), send_info))
                            .await
                            .map_err(into_io_error)?;
                    }
                }
            }
        }

        Ok(())
    }
}
