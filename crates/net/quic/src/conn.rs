use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
    ops,
    sync::Arc,
};

use hala_future::executor::future_spawn;
use hala_io::timeout;
use hala_udp::UdpSocket;
use quiche::RecvInfo;

use crate::{
    state::{QuicConnState, QuicConnectorState},
    Config,
};

/// Socket connection type for quic protocol.
#[must_use = "If return variable is not bound, the created stream will be dropped immediately"]
#[derive(Debug, Clone)]
pub struct QuicConn {
    state: QuicConnState,
    conn_counter: Arc<()>,
}

impl From<QuicConnState> for QuicConn {
    fn from(value: QuicConnState) -> Self {
        Self {
            state: value,
            conn_counter: Arc::new(()),
        }
    }
}

impl Drop for QuicConn {
    fn drop(&mut self) {
        // only this instance is aliving.
        if Arc::strong_count(&self.conn_counter) == 1 {
            let state = self.state.clone();

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
}

impl QuicConn {
    pub async fn connect<L: ToSocketAddrs, R: ToSocketAddrs>(
        laddrs: L,
        raddrs: R,
        config: &mut Config,
    ) -> io::Result<Self> {
        let udpsocket = UdpSocket::bind(laddrs)?;

        let mut lastest_error = None;

        for raddr in raddrs.to_socket_addrs()? {
            match Self::connect_with(&udpsocket, raddr, config).await {
                Err(err) => lastest_error = Some(err),
                Ok(conn) => {
                    event_loop::run_client_loop(udpsocket, conn.clone(), config.max_datagram_size);

                    return Ok(conn);
                }
            }
        }

        return Err(lastest_error.unwrap());
    }

    async fn connect_with(
        udp_socket: &UdpSocket,
        raddr: SocketAddr,
        config: &mut Config,
    ) -> io::Result<QuicConn> {
        let mut buf = vec![0; config.max_datagram_size];

        let laddr = udp_socket.local_addr()?;

        let mut connector_state = QuicConnectorState::new(config, laddr, raddr)?;

        loop {
            if let Some((send_size, send_info)) = connector_state.send(&mut buf)? {
                udp_socket.send_to(&buf[..send_size], send_info.to).await?;
            }

            if connector_state.is_closed() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("Connect to remove server timeout, raddr={:?}", raddr),
                ));
            }

            let send_timeout = connector_state.timeout();

            log::trace!("Connect send timeout={:?}", send_timeout);

            let (recv_size, raddr) =
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
                    from: raddr,
                    to: laddr,
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
}

/// Stream socket type for quic protocol.
#[must_use = "If return variable is not bound, the created stream will be dropped immediately"]
#[derive(Debug, Clone)]
pub struct QuicStream {
    conn: QuicConn,
    stream_id: u64,
    stream_counter: Arc<()>,
}

impl From<(QuicConn, u64)> for QuicStream {
    fn from(value: (QuicConn, u64)) -> Self {
        Self {
            conn: value.0,
            stream_id: value.1,
            stream_counter: Arc::default(),
        }
    }
}

impl Drop for QuicStream {
    fn drop(&mut self) {
        // only this instance is aliving.
        if Arc::strong_count(&self.stream_counter) == 1 {
            let conn = self.conn.clone();
            let stream_id = self.stream_id;

            future_spawn(async move {
                match conn.state.close_stream(stream_id).await {
                    Ok(_) => {
                        log::info!(
                            "quic conn, scid={:?}, dcide={:?}, stream_id={}, closed successfully",
                            conn.state.scid,
                            conn.state.dcid,
                            stream_id
                        );
                    }
                    Err(err) => {
                        log::info!(
                            "quic conn, scid={:?}, dcide={:?}, stream_id={} closed with error, {}",
                            conn.state.scid,
                            conn.state.dcid,
                            stream_id,
                            err
                        );
                    }
                }
            });
        }
    }
}

impl QuicStream {
    /// Send data to peer over this stream.
    pub async fn stream_send(&self, buf: &mut [u8], fin: bool) -> io::Result<usize> {
        self.conn.state.stream_send(self.stream_id, buf, fin).await
    }

    /// Recv data from peer over this stream. if successful, returns read data length and fin flag
    pub async fn stream_recv(&self, buf: &mut [u8]) -> io::Result<(usize, bool)> {
        self.conn.state.stream_recv(self.stream_id, buf).await
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
                .map(|r| r.map(|(read_size, _)| read_size))
        }
    }
}

mod event_loop {

    use super::*;

    pub(super) fn run_client_loop(udp_socket: UdpSocket, conn: QuicConn, max_datagram_size: usize) {
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
        udp_socket: Arc<UdpSocket>,
        conn: QuicConn,
        max_datagram_size: usize,
    ) -> io::Result<()> {
        let mut buf = vec![0; max_datagram_size];

        let laddr = udp_socket.local_addr()?;

        loop {
            let (recv_size, raddr) = udp_socket.recv_from(&mut buf).await?;

            conn.state
                .write(
                    &mut buf[..recv_size],
                    RecvInfo {
                        from: raddr,
                        to: laddr,
                    },
                )
                .await?;
        }
    }

    async fn run_client_send_loop(
        udp_socket: Arc<UdpSocket>,
        conn: QuicConn,
        max_datagram_size: usize,
    ) -> io::Result<()> {
        let mut buf = vec![0; max_datagram_size];

        loop {
            let (read_size, send_info) = conn.state.read(&mut buf).await?;

            udp_socket.send_to(&buf[..read_size], send_info.to).await?;
        }
    }
}
