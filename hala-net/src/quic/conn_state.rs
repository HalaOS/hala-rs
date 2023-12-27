use std::{
    collections::{HashSet, VecDeque},
    fmt::Debug,
    io,
    sync::Arc,
};

use future_mediator::{
    shared::{AsyncLocalShared, AsyncShared},
    LocalMediator,
};

use hala_io_util::local_io_spawn;
use quiche::{RecvInfo, SendInfo};

use crate::{errors::into_io_error, quic::QuicStream};

/// Quic connection state object
pub struct QuicConnState {
    stream_id_seed: u64,
    /// quiche connection instance.
    quiche_conn: quiche::Connection,
    /// Opened stream id set
    opened_streams: HashSet<u64>,
    /// Incoming stream deque.
    incoming_streams: VecDeque<u64>,
}

impl Debug for QuicConnState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuicConnState")
            .field("stream_id_seed", &self.stream_id_seed)
            .field("opened_streams", &self.opened_streams.len())
            .field("incoming_streams", &self.incoming_streams.len())
            .finish()
    }
}

impl QuicConnState {
    /// Create new `QuicConnState` from [`Connection`](quiche::Connection)
    pub fn new(quiche_conn: quiche::Connection, stream_id_seed: u64) -> Self {
        Self {
            stream_id_seed,
            quiche_conn,
            opened_streams: Default::default(),
            incoming_streams: Default::default(),
        }
    }
}

impl Drop for QuicConnState {
    fn drop(&mut self) {
        log::trace!("dropping conn={}", self.quiche_conn.trace_id());
    }
}

/// `QuicConnState` support event variant.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum QuicConnEvents {
    Send(String),
    Recv(String),
    StreamSend(String, u64),
    StreamRecv(String, u64),
    Accept(String),
    OpenStream,
}

async fn handle_accept(
    mediator: &LocalMediator<QuicConnEvents>,
    state: &mut QuicConnState,
    stream_id: u64,
) {
    if !state.opened_streams.contains(&stream_id) {
        log::trace!(
            "handle incoming, conn={:?}, stream={}",
            state.quiche_conn.trace_id(),
            stream_id
        );

        state.opened_streams.insert(stream_id);
        state.incoming_streams.push_back(stream_id);

        mediator
            .notify(QuicConnEvents::Accept(state.quiche_conn.trace_id().into()))
            .await;
    }
}

async fn handle_stream(mediator: &LocalMediator<QuicConnEvents>, state: &mut QuicConnState) {
    let mut events = vec![];

    for stream_id in state.quiche_conn.readable() {
        handle_accept(mediator, state, stream_id).await;

        events.push(QuicConnEvents::StreamSend(
            state.quiche_conn.trace_id().into(),
            stream_id,
        ));
    }

    mediator.notify_all(&events).await;

    for stream_id in state.quiche_conn.writable() {
        handle_accept(mediator, state, stream_id).await;

        events.push(QuicConnEvents::StreamSend(
            state.quiche_conn.trace_id().into(),
            stream_id,
        ));
    }

    mediator.notify_all(&events).await;
}

async fn handle_close(mediator: &LocalMediator<QuicConnEvents>) {
    mediator.notify_any().await;
}

/// Quic connection state object
#[derive(Clone)]
pub struct AsyncQuicConnState {
    conn_state: AsyncLocalShared<QuicConnState>,
    /// core inner state.
    pub(crate) mediator: LocalMediator<QuicConnEvents>,

    /// String type trace id.
    pub trace_id: Arc<String>,
}

impl AsyncQuicConnState {
    pub fn new(quiche_conn: quiche::Connection, stream_id_seed: u64) -> Self {
        Self {
            trace_id: Arc::new(quiche_conn.trace_id().to_owned()),
            conn_state: QuicConnState::new(quiche_conn, stream_id_seed).into(),
            mediator: LocalMediator::new_with("mediator: quic_conn_state"),
        }
    }

    /// Create new future for send connection data
    pub async fn send<'a>(&self, buf: &'a mut [u8]) -> io::Result<(usize, SendInfo)> {
        let event = QuicConnEvents::Recv(self.trace_id.to_string());

        let mut conn_state = self.conn_state.lock_mut_wait().await;

        loop {
            if conn_state.quiche_conn.is_closed() {
                handle_close(&self.mediator).await;

                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!("quic conn closed, id={}", conn_state.quiche_conn.trace_id()),
                ));
            }

            match conn_state.quiche_conn.send(buf) {
                Ok((recv_size, send_info)) => {
                    self.mediator
                        .notify(QuicConnEvents::Recv(self.trace_id.to_string()))
                        .await;

                    handle_stream(&self.mediator, &mut conn_state).await;

                    return Ok((recv_size, send_info));
                }
                Err(quiche::Error::Done) => {
                    log::trace!("{:?} done ", event);

                    if conn_state.quiche_conn.is_closed() {
                        handle_close(&self.mediator).await;

                        return Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            format!("conn={} closed", conn_state.quiche_conn.trace_id()),
                        ));
                    }

                    let timeout = conn_state.quiche_conn.timeout();

                    conn_state = hala_io_util::timeout(
                        async {
                            let conn_state =
                                self.mediator.event_wait(conn_state, event.clone()).await;

                            Ok(conn_state)
                        },
                        timeout,
                    )
                    .await?;

                    continue;
                }
                Err(err) => {
                    if conn_state.quiche_conn.is_closed() {
                        handle_close(&self.mediator).await;
                    }

                    return Err(into_io_error(err));
                }
            }
        }
    }

    /// Create new future for recv connection data
    pub async fn recv<'a>(&self, buf: &'a mut [u8], recv_info: RecvInfo) -> io::Result<usize> {
        let event = QuicConnEvents::Recv(self.trace_id.to_string());

        let mut conn_state = self.conn_state.lock_mut_wait().await;

        loop {
            if conn_state.quiche_conn.is_closed() {
                handle_close(&self.mediator).await;

                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!("quic conn closed, id={}", conn_state.quiche_conn.trace_id()),
                ));
            }

            match conn_state.quiche_conn.recv(buf, recv_info.clone()) {
                Ok(recv_size) => {
                    self.mediator
                        .notify(QuicConnEvents::Send(self.trace_id.to_string()))
                        .await;

                    handle_stream(&self.mediator, &mut conn_state).await;

                    return Ok(recv_size);
                }
                Err(quiche::Error::Done) => {
                    log::trace!("{:?} done ", event);

                    if conn_state.quiche_conn.is_closed() {
                        handle_close(&self.mediator).await;

                        return Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            format!("conn={} closed", conn_state.quiche_conn.trace_id()),
                        ));
                    }

                    conn_state = self.mediator.event_wait(conn_state, event.clone()).await;

                    continue;
                }
                Err(err) => {
                    if conn_state.quiche_conn.is_closed() {
                        handle_close(&self.mediator).await;
                    }

                    return Err(into_io_error(err));
                }
            }
        }
    }

    /// Create new future for send stream data
    pub async fn stream_send<'a>(
        &self,
        stream_id: u64,
        buf: &'a [u8],
        fin: bool,
    ) -> io::Result<usize> {
        let event = QuicConnEvents::StreamSend(self.trace_id.to_string(), stream_id);

        let mut conn_state = self.conn_state.lock_mut_wait().await;

        loop {
            if conn_state.quiche_conn.is_closed() {
                handle_close(&self.mediator).await;

                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!("quic conn closed, id={}", conn_state.quiche_conn.trace_id()),
                ));
            }

            match conn_state.quiche_conn.stream_send(stream_id, buf, fin) {
                Ok(recv_size) => {
                    self.mediator
                        .notify(QuicConnEvents::Send(self.trace_id.to_string()))
                        .await;

                    if fin {
                        log::trace!("{:?}, status=fin", event);
                    }

                    return Ok(recv_size);
                }
                Err(quiche::Error::Done) => {
                    log::trace!(
                        "StreamSend({}, {}) done ",
                        self.trace_id.to_string(),
                        stream_id
                    );

                    if conn_state.quiche_conn.is_closed() {
                        handle_close(&self.mediator).await;

                        return Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            format!("conn={} closed", conn_state.quiche_conn.trace_id()),
                        ));
                    }

                    conn_state = self.mediator.event_wait(conn_state, event.clone()).await;

                    continue;
                }
                Err(err) => {
                    if conn_state.quiche_conn.is_closed() {
                        handle_close(&self.mediator).await;
                    }

                    return Err(into_io_error(err));
                }
            }
        }
    }

    /// Create new future for recv stream data
    pub async fn stream_recv<'a>(
        &self,
        stream_id: u64,
        buf: &'a mut [u8],
    ) -> io::Result<(usize, bool)> {
        let event = QuicConnEvents::StreamRecv(self.trace_id.to_string(), stream_id);

        let mut conn_state = self.conn_state.lock_mut_wait().await;

        loop {
            if conn_state.quiche_conn.is_closed() {
                handle_close(&self.mediator).await;

                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!("quic conn closed, id={}", conn_state.quiche_conn.trace_id()),
                ));
            }

            match conn_state.quiche_conn.stream_recv(stream_id, buf) {
                Ok((read_size, fin)) => {
                    self.mediator
                        .notify_all(&[
                            QuicConnEvents::Send(self.trace_id.to_string()),
                            QuicConnEvents::Recv(self.trace_id.to_string()),
                        ])
                        .await;

                    if fin {
                        log::trace!("{:?}, status=fin", event);
                    }

                    return Ok((read_size, fin));
                }
                Err(quiche::Error::Done) => {
                    log::trace!(
                        "StreamSend({}, {}) done ",
                        self.trace_id.to_string(),
                        stream_id
                    );

                    if conn_state.quiche_conn.is_closed() {
                        handle_close(&self.mediator).await;

                        return Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            format!("conn={} closed", conn_state.quiche_conn.trace_id()),
                        ));
                    }

                    conn_state = self.mediator.event_wait(conn_state, event.clone()).await;

                    continue;
                }
                Err(err) => {
                    if conn_state.quiche_conn.is_closed() {
                        handle_close(&self.mediator).await;
                    }

                    return Err(into_io_error(err));
                }
            }
        }
    }

    /// Open new stream to communicate with remote peer.
    pub async fn open_stream(&self) -> io::Result<QuicStream> {
        let mut state = self.conn_state.lock_mut_wait().await;

        if state.quiche_conn.is_closed() {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                format!("quic conn closed, id={}", state.quiche_conn.trace_id()),
            ));
        }

        let stream_id = state.stream_id_seed;
        state.stream_id_seed += 4;

        self.mediator
            .notify(QuicConnEvents::Send(self.trace_id.to_string()))
            .await;

        Ok(QuicStream::new(stream_id, self.clone()))
    }

    pub fn close_stream(&self, stream_id: u64) {
        let this = self.clone();

        local_io_spawn(async move {
            this.stream_send(stream_id, b"", true).await?;

            Ok(())
        })
        .unwrap();
    }

    pub(super) async fn is_stream_closed(&self, stream_id: u64) -> bool {
        self.conn_state
            .lock_wait()
            .await
            .quiche_conn
            .stream_finished(stream_id)
    }

    /// Close connection.
    pub(super) async fn close(&self, app: bool, err: u64, reason: &[u8]) -> io::Result<()> {
        self.conn_state
            .lock_mut_wait()
            .await
            .quiche_conn
            .close(app, err, reason)
            .map_err(into_io_error)
    }

    pub(super) async fn is_closed(&self) -> bool {
        self.conn_state.lock_wait().await.quiche_conn.is_closed()
    }

    pub async fn accept(&self) -> Option<QuicStream> {
        let event = QuicConnEvents::Accept(self.trace_id.to_string());

        let mut conn_state = self.conn_state.lock_mut_wait().await;

        loop {
            if conn_state.quiche_conn.is_closed() {
                log::trace!("{:?}, conn_status=closed", event);
                return None;
            }

            if let Some(stream_id) = conn_state.incoming_streams.pop_front() {
                return Some(QuicStream::new(stream_id, self.clone()));
            } else {
                conn_state = self.mediator.event_wait(conn_state, event.clone()).await;
            }
        }
    }
}
