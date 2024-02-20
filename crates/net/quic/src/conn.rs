use std::{
    fmt::Debug,
    io,
    net::{SocketAddr, ToSocketAddrs},
    ops,
    sync::Arc,
};

use hala_future::executor::future_spawn;
use hala_io::timeout;
use hala_udp::UdpGroup;
use quiche::{ConnectionId, RecvInfo};
use rand::{
    seq::{IteratorRandom, SliceRandom},
    thread_rng,
};

use crate::{
    state::{QuicConnState, QuicConnectorState},
    Config,
};

struct QuicConnDrop(QuicConnState);

impl Drop for QuicConnDrop {
    fn drop(&mut self) {
        let state = self.0.clone();

        future_spawn(async move {
            match state.close(false, 0, b"").await {
                Ok(_) => {
                    log::info!(
                        "quic conn, scid={:?}, dcide={:?} closed successfully",
                        state.scid,
                        state.dcid
                    );
                }
                Err(err) => {
                    log::info!(
                        "quic conn, scid={:?}, dcide={:?} closed with error, {}",
                        state.scid,
                        state.dcid,
                        err
                    );
                }
            }
        });
    }
}

/// Socket connection type for quic protocol.
#[must_use = "If return variable is not bound, the created stream will be dropped immediately"]
#[derive(Clone)]
pub struct QuicConn {
    state: QuicConnState,
    _conn_counter: Arc<QuicConnDrop>,
}

impl Debug for QuicConn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.state)
    }
}

impl From<QuicConnState> for QuicConn {
    fn from(value: QuicConnState) -> Self {
        Self {
            _conn_counter: Arc::new(QuicConnDrop(value.clone())),
            state: value,
        }
    }
}

impl QuicConn {
    pub async fn connect<L: ToSocketAddrs, R: ToSocketAddrs>(
        laddrs: L,
        raddrs: R,
        config: &mut Config,
    ) -> io::Result<Self> {
        let udpsocket = UdpGroup::bind(laddrs)?;

        let mut lastest_error = None;

        let mut raddrs = raddrs.to_socket_addrs()?.collect::<Vec<_>>();

        raddrs.shuffle(&mut thread_rng());

        for raddr in raddrs {
            match Self::connect_with(&udpsocket, raddr, config).await {
                Err(err) => lastest_error = Some(err),
                Ok(conn) => {
                    log::info!("connect to {:?} successfully, {:?}", raddr, conn);
                    event_loop::run_client_loop(udpsocket, conn.clone(), config.max_datagram_size);

                    return Ok(conn);
                }
            }
        }

        return Err(lastest_error.unwrap());
    }

    async fn connect_with(
        udp_socket: &UdpGroup,
        raddr: SocketAddr,
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
                return Ok(QuicConn::from(QuicConnState::from(connector_state)));
            }
        }
    }

    /// Open one new outgoing stream.
    pub async fn open_stream(&self) -> io::Result<QuicStream> {
        let stream_id = self.state.open_stream().await?;

        Ok((self.clone(), stream_id).into())
    }

    /// Accept ine incoming stream. returns `None` if this connection had been closed.
    pub async fn accept_stream(&self) -> Option<QuicStream> {
        if let Some(stream_id) = self.state.accept().await {
            Some((self.clone(), stream_id).into())
        } else {
            None
        }
    }

    pub async fn to_quiche_conn(&self) -> impl ops::Deref<Target = quiche::Connection> + '_ {
        self.state.to_quiche_conn().await
    }

    /// Returns the number of bidirectional streams that can be created
    /// before the peer's stream count limit is reached.
    pub async fn peer_streams_left_bidi(&self) -> u64 {
        self.state.peer_streams_left_bidi().await
    }

    /// The source id of this connection.
    pub fn source_id(&self) -> &ConnectionId<'static> {
        &self.state.scid
    }
    /// The destination id of this connection.
    pub fn destination_id(&self) -> &ConnectionId<'static> {
        &self.state.dcid
    }

    /// Manual close this connection.
    pub async fn close(&self) -> io::Result<()> {
        self.state.close(false, 0, b"User manual closed").await
    }

    /// Get the closed flag of this connection.
    pub async fn is_closed(&self) -> bool {
        self.state.is_closed().await
    }
}

struct QuicStreamDrop(u64, QuicConn);

impl Drop for QuicStreamDrop {
    fn drop(&mut self) {
        let conn = self.1.clone();
        let stream_id = self.0;

        future_spawn(async move {
            _ = conn.state.close_stream(stream_id).await;
        });
    }
}

/// Stream socket type for quic protocol.
#[must_use = "If return variable is not bound, the created stream will be dropped immediately"]
#[derive(Clone)]
pub struct QuicStream {
    pub conn: QuicConn,
    pub stream_id: u64,
    _stream_counter: Arc<QuicStreamDrop>,
}

impl Debug for QuicStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}, stream_id={}", self.conn, self.stream_id)
    }
}

impl From<(QuicConn, u64)> for QuicStream {
    fn from(value: (QuicConn, u64)) -> Self {
        Self {
            _stream_counter: Arc::new(QuicStreamDrop(value.1, value.0.clone())),
            conn: value.0,
            stream_id: value.1,
        }
    }
}

impl QuicStream {
    /// Send data to peer over this stream.
    pub async fn stream_send(&self, buf: &[u8], fin: bool) -> io::Result<usize> {
        self.conn.state.stream_send(self.stream_id, buf, fin).await
    }

    /// Recv data from peer over this stream. if successful, returns read data length and fin flag
    pub async fn stream_recv(&self, buf: &mut [u8]) -> io::Result<(usize, bool)> {
        self.conn.state.stream_recv(self.stream_id, buf).await
    }

    /// Shutdown stream read operations.
    pub async fn stream_shutdown(&self) -> io::Result<()> {
        self.conn.state.stream_shutdown(self.stream_id, 0).await
    }

    pub fn to_id(&self) -> u64 {
        self.stream_id
    }
}

#[cfg(feature = "futures_async_api_support")]
pub mod async_read_write {
    use super::*;

    use futures::{AsyncRead, AsyncWrite, FutureExt};

    impl AsyncWrite for QuicStream {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &[u8],
        ) -> std::task::Poll<io::Result<usize>> {
            Box::pin(self.conn.state.stream_send(self.stream_id, buf, false)).poll_unpin(cx)
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_close(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            Box::pin(self.conn.state.close_stream(self.stream_id))
                .poll_unpin(cx)
                .map(|_| Ok(()))
        }
    }

    impl AsyncRead for QuicStream {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &mut [u8],
        ) -> std::task::Poll<io::Result<usize>> {
            Box::pin(self.conn.state.stream_recv(self.stream_id, buf))
                .poll_unpin(cx)
                .map(|r| match r {
                    Ok((readsize, _)) => Ok(readsize),
                    Err(err) => {
                        if err.kind() == io::ErrorKind::BrokenPipe {
                            Ok(0)
                        } else {
                            Err(err)
                        }
                    }
                })
        }
    }
}

mod event_loop {

    use hala_udp::PathInfo;

    use super::*;

    pub(super) fn run_client_loop(udp_socket: UdpGroup, conn: QuicConn, max_datagram_size: usize) {
        let udp_socket = Arc::new(udp_socket);

        let udp_socket_cloned = udp_socket.clone();

        let conn_cloned = conn.clone();

        future_spawn(async move {
            let conn_scid = conn_cloned.state.scid.clone().into_owned();

            let conn_dcid = conn_cloned.state.dcid.clone().into_owned();

            match run_client_recv_loop(udp_socket_cloned, conn_cloned, max_datagram_size).await {
                Ok(_) => {
                    log::trace!(
                        "quic conn, scid={:?}, dcid={:?} recv loop stop.",
                        conn_scid,
                        conn_dcid
                    );
                }
                Err(err) => {
                    log::trace!(
                        "quic conn, scid={:?}, dcid={:?} recv loop stop with err, {}",
                        conn_scid,
                        conn_dcid,
                        err
                    );
                }
            }
        });

        future_spawn(async move {
            let conn_scid = conn.state.scid.clone().into_owned();

            let conn_dcid = conn.state.dcid.clone().into_owned();

            match run_client_send_loop(udp_socket, conn, max_datagram_size).await {
                Ok(_) => {
                    log::trace!(
                        "quic conn, scid={:?}, dcid={:?} send loop stop.",
                        conn_scid,
                        conn_dcid
                    );
                }
                Err(err) => {
                    log::trace!(
                        "quic conn, scid={:?}, dcid={:?} send loop stop with err, {}",
                        conn_scid,
                        conn_dcid,
                        err
                    );
                }
            }
        });
    }

    async fn run_client_recv_loop(
        udp_socket: Arc<UdpGroup>,
        conn: QuicConn,
        max_datagram_size: usize,
    ) -> io::Result<()> {
        let mut buf = vec![0; max_datagram_size];

        loop {
            let (recv_size, path_info) = udp_socket.recv_from(&mut buf).await?;

            log::trace!(
                "Quic client recv data, len={}, path_info={:?}",
                recv_size,
                path_info
            );

            conn.state
                .write(
                    &mut buf[..recv_size],
                    RecvInfo {
                        from: path_info.from,
                        to: path_info.to,
                    },
                )
                .await?;
        }
    }

    async fn run_client_send_loop(
        udp_socket: Arc<UdpGroup>,
        conn: QuicConn,
        max_datagram_size: usize,
    ) -> io::Result<()> {
        let mut buf = vec![0; max_datagram_size];

        loop {
            let (read_size, send_info) = conn.state.read(&mut buf).await?;

            let path_info = PathInfo {
                from: send_info.from,
                to: send_info.to,
            };

            let send_size = udp_socket
                .send_to_on_path(&buf[..read_size], path_info)
                .await?;

            log::trace!(
                "Quic client send data, len={}, path_info={:?}",
                send_size,
                path_info
            );
        }
    }
}
