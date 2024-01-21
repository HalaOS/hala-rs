use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use futures::{
    channel::mpsc::{self, Receiver, Sender},
    SinkExt, StreamExt,
};
use hala_future::executor::future_spawn;
use hala_io::{
    bytes::{Bytes, BytesMut},
    timeout, ReadBuf,
};
use hala_udp::{PathInfo, UdpGroup};
use quiche::{ConnectionId, RecvInfo, SendInfo};
use rand::{seq::IteratorRandom, thread_rng};
use ring::rand::{SecureRandom, SystemRandom};

use crate::{errors::into_io_error, Config};

use super::{QuicConnState, QuicConnStateEvent, QuicIncomingStream, QuicStreamBuf};

/// Quic client connector
struct QuicConnectorState {
    /// source connection id.
    pub(super) quiche_conn: quiche::Connection,
    pub(super) ping_timeout: Duration,
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

        Ok(Self {
            quiche_conn,
            ping_timeout: config.ping_timeout,
        })
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

    /// Check if underly connection is closed.
    pub fn is_closed(&self) -> bool {
        self.quiche_conn.is_closed()
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

/// Quic connection type.
pub struct QuicConn {
    max_state_cache_len: usize,
    stream_id_next: Arc<AtomicU64>,
    scid: ConnectionId<'static>,
    dcid: ConnectionId<'static>,
    event_sender: Sender<QuicConnStateEvent>,
    conn_data_receiver: Option<Receiver<(Bytes, SendInfo)>>,
    incoming_stream_receiver: Receiver<QuicIncomingStream>,
}

impl QuicConn {
    pub fn pair(
        quiche_conn: quiche::Connection,
        send_ping_timeout: Duration,
        max_state_cache_len: usize,
    ) -> (QuicConn, QuicConnState) {
        let (event_sender, event_receiver) = mpsc::channel(max_state_cache_len);

        let (conn_data_sender, conn_data_receiver) = mpsc::channel(max_state_cache_len);

        let (incoming_stream_sender, incoming_stream_receiver) = mpsc::channel(max_state_cache_len);

        let scid = quiche_conn.source_id().clone().into_owned();
        let dcid = quiche_conn.destination_id().clone().into_owned();

        let stream_id_next = if quiche_conn.is_server() {
            Arc::new(AtomicU64::new(5))
        } else {
            Arc::new(AtomicU64::new(4))
        };

        let state = QuicConnState::new(
            max_state_cache_len,
            quiche_conn,
            send_ping_timeout,
            event_receiver,
            conn_data_sender,
            incoming_stream_sender,
        );

        let conn = QuicConn {
            max_state_cache_len,
            stream_id_next,
            scid,
            dcid,
            event_sender,
            conn_data_receiver: Some(conn_data_receiver),
            incoming_stream_receiver,
        };

        (conn, state)
    }

    pub async fn connect<L: ToSocketAddrs, R: ToSocketAddrs>(
        laddrs: L,
        raddrs: R,
        max_stream_recv_packet_cache_len: usize,
        config: &mut Config,
    ) -> io::Result<Self> {
        let udpsocket = UdpGroup::bind(laddrs)?;

        let mut lastest_error = None;

        for raddr in raddrs.to_socket_addrs()? {
            match Self::connect_with(&udpsocket, raddr, max_stream_recv_packet_cache_len, config)
                .await
            {
                Err(err) => lastest_error = Some(err),
                Ok(mut conn) => {
                    conn.start_client_event_loops(udpsocket);

                    return Ok(conn);
                }
            }
        }

        return Err(lastest_error.unwrap());
    }

    async fn connect_with(
        udp_socket: &UdpGroup,
        raddr: SocketAddr,
        max_stream_recv_packet_cache_len: usize,
        config: &mut Config,
    ) -> io::Result<QuicConn> {
        let mut buf = vec![0; config.max_datagram_size];

        let laddr = *udp_socket.local_addrs().choose(&mut thread_rng()).unwrap();

        let mut connector_state = QuicConnectorState::new(config, laddr, raddr)?;

        loop {
            if let Some((send_size, send_info)) = connector_state.send(&mut buf)? {
                udp_socket
                    .send_to_on_path(
                        &buf[..send_size],
                        hala_udp::PathInfo {
                            from: send_info.from,
                            to: send_info.to,
                        },
                    )
                    .await?;
            }

            if connector_state.is_closed() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("Connect to remove server timeout, raddr={:?}", raddr),
                ));
            }

            let send_timeout = connector_state.timeout();

            log::trace!("Connect send timeout={:?}", send_timeout);

            let (recv_size, path_info) =
                match timeout(udp_socket.recv_from(&mut buf), send_timeout).await {
                    Ok(r) => r,
                    Err(err) if err.kind() == io::ErrorKind::TimedOut => {
                        connector_state.on_timeout();
                        continue;
                    }
                    Err(err) => return Err(err),
                };

            connector_state.recv(
                &mut buf[..recv_size],
                RecvInfo {
                    from: path_info.from,
                    to: path_info.to,
                },
            )?;

            if connector_state.is_established() {
                let (conn, state) = Self::pair(
                    connector_state.quiche_conn,
                    connector_state.ping_timeout,
                    max_stream_recv_packet_cache_len,
                );

                state.start();

                return Ok(conn);
            }
        }
    }
}

impl QuicConn {
    /// Accept new incoming stream.
    pub async fn accept(&mut self) -> Option<QuicStream> {
        self.incoming_stream_receiver
            .next()
            .await
            .map(|incoming_stream| QuicStream {
                scid: self.scid.clone(),
                dcid: self.dcid.clone(),
                event_sender: self.event_sender.clone(),
                incoming_stream,
                stream_recv_buf: Default::default(),
            })
    }

    /// Open a new bidi stream.
    pub async fn open(&mut self) -> io::Result<QuicStream> {
        let stream_id = self.stream_id_next.fetch_add(2, Ordering::Relaxed);

        let (stream_data_sender, stream_data_receiver) = mpsc::channel(self.max_state_cache_len);

        self.event_sender
            .send(QuicConnStateEvent::OpenStream {
                stream_id,
                stream_data_sender,
            })
            .await
            .map_err(into_io_error)?;

        Ok(QuicStream {
            scid: self.scid.clone(),
            dcid: self.dcid.clone(),
            event_sender: self.event_sender.clone(),
            incoming_stream: QuicIncomingStream {
                stream_data_receiver,
                stream_id,
            },
            stream_recv_buf: Default::default(),
        })
    }
}

impl QuicConn {
    fn start_client_event_loops(&mut self, udp_socket: UdpGroup) {
        let udp_socket = Arc::new(udp_socket);

        future_spawn(Self::send_loop(
            self.scid.clone(),
            self.dcid.clone(),
            self.conn_data_receiver.take().unwrap(),
            udp_socket.clone(),
        ));

        future_spawn(Self::recv_loop(
            self.scid.clone(),
            self.dcid.clone(),
            self.event_sender.clone(),
            udp_socket.clone(),
        ));
    }

    async fn send_loop(
        scid: ConnectionId<'static>,
        dcid: ConnectionId<'static>,
        mut receiver: Receiver<(Bytes, SendInfo)>,
        udp_socket: Arc<UdpGroup>,
    ) {
        log::trace!(
            "QuicConn, scid={:?}, dcid={:?}, start send loop",
            scid,
            dcid
        );

        while let Some((buf, send_info)) = receiver.next().await {
            match udp_socket
                .send_to_on_path(
                    &buf,
                    PathInfo {
                        from: send_info.from,
                        to: send_info.to,
                    },
                )
                .await
            {
                Ok(len) => {
                    assert_eq!(len, buf.len());
                }
                Err(err) => {
                    log::trace!(
                        "QuicConn, scid={:?}, dcid={:?}, send loop stopped by error, {}",
                        scid,
                        dcid,
                        err
                    );

                    return;
                }
            }
        }

        log::trace!(
            "QuicConn, scid={:?}, dcid={:?}, send loop stopped",
            scid,
            dcid
        );
    }

    async fn recv_loop(
        scid: ConnectionId<'static>,
        dcid: ConnectionId<'static>,
        mut sender: Sender<QuicConnStateEvent>,
        udp_socket: Arc<UdpGroup>,
    ) {
        log::trace!(
            "QuicConn, scid={:?}, dcid={:?}, start recv loop",
            scid,
            dcid
        );

        loop {
            let mut buf = ReadBuf::with_capacity(1370);

            match udp_socket.recv_from(buf.as_mut()).await {
                Ok((read_size, path_info)) => {
                    if sender
                        .send(QuicConnStateEvent::Recv {
                            buf: buf.into_bytes_mut(Some(read_size)),
                            recv_info: RecvInfo {
                                from: path_info.from,
                                to: path_info.to,
                            },
                        })
                        .await
                        .is_err()
                    {
                        // conn had been dropped.
                        break;
                    }
                }
                Err(err) => {
                    log::trace!(
                        "QuicConn, scid={:?}, dcid={:?}, recv loop stopped by error, {}",
                        scid,
                        dcid,
                        err
                    );

                    return;
                }
            }
        }

        log::trace!(
            "QuicConn, scid={:?}, dcid={:?}, recv loop stopped",
            scid,
            dcid
        );
    }
}

pub struct QuicStream {
    pub scid: ConnectionId<'static>,
    pub dcid: ConnectionId<'static>,
    event_sender: Sender<QuicConnStateEvent>,
    incoming_stream: QuicIncomingStream,
    stream_recv_buf: QuicStreamBuf,
}

impl QuicStream {
    /// Get quic stream id.
    pub fn stream_id(&self) -> u64 {
        self.incoming_stream.stream_id
    }

    /// Send data to peer over this stream.
    pub async fn stream_send(&mut self, buf: &[u8], fin: bool) -> io::Result<usize> {
        self.event_sender
            .send(QuicConnStateEvent::StreamSend {
                stream_id: self.incoming_stream.stream_id,
                buf: BytesMut::from(buf),
                fin,
            })
            .await
            .map_err(into_io_error)?;

        Ok(buf.len())
    }

    /// Read data from peer over the stream.
    pub async fn stream_recv(&mut self, buf: &mut [u8]) -> io::Result<(usize, bool)> {
        let (read_size, fin) = self.stream_recv_buf.read(buf);

        if read_size > 0 || fin {
            return Ok((read_size, fin));
        }

        if let Some(mut recv_buf) = self.incoming_stream.stream_data_receiver.next().await {
            let (read_size, fin) = recv_buf.read(buf);

            self.stream_recv_buf.append_buf(recv_buf);

            return Ok((read_size, fin));
        } else {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "Stream closed"));
        }
    }
}
