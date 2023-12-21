use std::{
    collections::{HashSet, VecDeque},
    io,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    task::Poll,
};

use future_mediator::{LocalMediator, Mediator, Shared};
use futures::FutureExt;
use hala_io_util::{get_poller, Sleep};
use quiche::{RecvInfo, SendInfo};

use crate::{errors::into_io_error, quic::QuicStream};

/// Quic connection state object
pub struct QuicConnState {
    /// quiche connection instance.
    quiche_conn: quiche::Connection,
    /// Opened stream id set
    opened_streams: HashSet<u64>,
    /// Incoming stream deque.
    incoming_streams: VecDeque<u64>,
}

impl QuicConnState {
    /// Create new `QuicConnState` from [`Connection`](quiche::Connection)
    pub fn new(quiche_conn: quiche::Connection) -> Self {
        Self {
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
}

fn handle_accept(cx: &mut Shared<QuicConnState, QuicConnEvents>, stream_id: u64) {
    if !cx.opened_streams.contains(&stream_id) {
        cx.opened_streams.insert(stream_id);
        cx.incoming_streams.push_back(stream_id);

        cx.notify(QuicConnEvents::Accept(cx.quiche_conn.trace_id().into()));
    }
}

fn handle_stream(cx: &mut Shared<QuicConnState, QuicConnEvents>) {
    for stream_id in cx.quiche_conn.readable() {
        handle_accept(cx, stream_id);
        cx.notify(QuicConnEvents::StreamRecv(
            cx.quiche_conn.trace_id().into(),
            stream_id,
        ));
    }

    for stream_id in cx.quiche_conn.writable() {
        handle_accept(cx, stream_id);

        cx.notify(QuicConnEvents::StreamSend(
            cx.quiche_conn.trace_id().into(),
            stream_id,
        ));
    }
}

fn handle_close(cx: &mut Shared<QuicConnState, QuicConnEvents>) {
    let ids = cx.opened_streams.iter().map(|id| *id).collect::<Vec<_>>();

    for stream_id in &ids {
        cx.notify(QuicConnEvents::StreamRecv(
            cx.quiche_conn.trace_id().into(),
            *stream_id,
        ));
        cx.notify(QuicConnEvents::StreamSend(
            cx.quiche_conn.trace_id().into(),
            *stream_id,
        ));
    }

    cx.notify(QuicConnEvents::Recv(cx.quiche_conn.trace_id().into()));
    cx.notify(QuicConnEvents::Send(cx.quiche_conn.trace_id().into()));
    cx.notify(QuicConnEvents::Accept(cx.quiche_conn.trace_id().into()));
}

fn handle_stream_close(cx: &mut Shared<QuicConnState, QuicConnEvents>, stream_id: u64) {
    cx.notify(QuicConnEvents::StreamRecv(
        cx.quiche_conn.trace_id().into(),
        stream_id,
    ));
    cx.notify(QuicConnEvents::StreamSend(
        cx.quiche_conn.trace_id().into(),
        stream_id,
    ));
}

/// Quic connection state object
#[derive(Clone)]
pub struct AsyncQuicConnState {
    /// core inner state.
    pub(crate) state: LocalMediator<QuicConnState, QuicConnEvents>,
    /// stream id generator seed
    stream_id_seed: Arc<AtomicU64>,
    /// String type trace id.
    pub trace_id: Arc<String>,
}

impl AsyncQuicConnState {
    pub fn new(quiche_conn: quiche::Connection, stream_id_seed: u64) -> Self {
        Self {
            trace_id: Arc::new(quiche_conn.trace_id().to_owned()),
            state: LocalMediator::new(QuicConnState::new(quiche_conn)),
            stream_id_seed: Arc::new(stream_id_seed.into()),
        }
    }

    /// Create new future for send connection data
    pub async fn send<'a>(&self, buf: &'a mut [u8]) -> io::Result<(usize, SendInfo)> {
        let mut sleep: Option<Sleep> = None;

        self.state
            .on_fn(
                QuicConnEvents::Send(self.trace_id.to_string()),
                |state, cx| {
                    if state.quiche_conn.is_closed() {
                        handle_close(state);

                        return Poll::Ready(Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            format!("conn={} closed", state.quiche_conn.trace_id()),
                        )));
                    }

                    if let Some(mut sleep) = sleep.take() {
                        match sleep.poll_unpin(cx) {
                            Poll::Ready(_) => {
                                log::trace!(
                                    "QuicConnSend(conn_id={}) on_timeout",
                                    state.quiche_conn.trace_id()
                                );
                                state.quiche_conn.on_timeout();
                            }
                            Poll::Pending => {}
                        }
                    }

                    loop {
                        match state.quiche_conn.send(buf) {
                            Ok((send_size, send_info)) => {
                                log::trace!(
                                    "QuicConnSend(conn_id={}), send_size={}, send_info={:?}",
                                    state.quiche_conn.trace_id(),
                                    send_size,
                                    send_info
                                );

                                handle_stream(state);

                                return Poll::Ready(Ok((send_size, send_info)));
                            }
                            Err(quiche::Error::Done) => {
                                if let Some(expired) = state.quiche_conn.timeout() {
                                    log::trace!(
                                        "QuicConnSend(conn_id={}) add timeout({:?})",
                                        state.quiche_conn.trace_id(),
                                        expired
                                    );

                                    let mut timeout = Sleep::new_with(get_poller()?, expired)?;

                                    match timeout.poll_unpin(cx) {
                                        Poll::Ready(_) => {
                                            log::trace!(
                                                "QuicConnSend(conn_id={}) on_timeout",
                                                state.quiche_conn.trace_id()
                                            );

                                            state.quiche_conn.on_timeout();
                                            continue;
                                        }
                                        _ => {
                                            sleep = Some(timeout);
                                        }
                                    }
                                }

                                log::trace!(
                                    "QuicConnSend(conn_id={}),stats={:?}, done, is_closed={:?}",
                                    state.quiche_conn.trace_id(),
                                    state.quiche_conn.stats(),
                                    state.quiche_conn.is_closed()
                                );

                                if state.quiche_conn.is_closed() {
                                    handle_close(state);
                                }

                                return Poll::Pending;
                            }
                            Err(err) => return Poll::Ready(Err(into_io_error(err))),
                        }
                    }
                },
            )
            .await
    }

    /// Create new future for recv connection data
    pub async fn recv<'a>(&self, buf: &'a mut [u8], recv_info: RecvInfo) -> io::Result<usize> {
        self.state
            .on_fn(
                QuicConnEvents::Recv(self.trace_id.to_string()),
                |state, _| {
                    if state.quiche_conn.is_closed() {
                        handle_close(state);
                        return Poll::Ready(Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            format!("conn={} closed", state.quiche_conn.trace_id()),
                        )));
                    }

                    let recv_info = recv_info;

                    match state.quiche_conn.recv(buf, recv_info) {
                        Ok(recv_size) => {
                            log::trace!(
                                "conn={} recv data, len={}",
                                state.quiche_conn.trace_id(),
                                recv_size
                            );

                            if state.quiche_conn.is_closed() {
                                handle_close(state);
                            } else {
                                handle_stream(state);
                            }

                            return Poll::Ready(Ok(recv_size));
                        }
                        Err(quiche::Error::Done) => {
                            if state.quiche_conn.is_closed() {
                                handle_close(state);
                            }
                            return Poll::Pending;
                        }
                        Err(err) => {
                            if state.quiche_conn.is_closed() {
                                handle_close(state);
                            }
                            return Poll::Ready(Err(into_io_error(err)));
                        }
                    }
                },
            )
            .await
    }

    /// Create new future for send stream data
    pub async fn stream_send<'a>(
        &self,
        stream_id: u64,
        buf: &'a [u8],
        fin: bool,
    ) -> io::Result<usize> {
        self.state
            .on_fn(
                QuicConnEvents::StreamSend(self.trace_id.to_string(), stream_id),
                |state, _| {
                    if state.quiche_conn.is_closed() {
                        handle_stream_close(state, stream_id);

                        return Poll::Ready(Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            format!("conn={} closed", state.quiche_conn.trace_id()),
                        )));
                    }

                    match state.quiche_conn.stream_send(stream_id, buf, fin) {
                        Ok(recv_size) => {
                            state.notify(QuicConnEvents::Send(self.trace_id.to_string()));

                            return Poll::Ready(Ok(recv_size));
                        }
                        Err(quiche::Error::Done) => {
                            if state.quiche_conn.is_closed() {
                                handle_stream_close(state, stream_id);

                                return Poll::Ready(Err(io::Error::new(
                                    io::ErrorKind::BrokenPipe,
                                    format!("conn={} closed", state.quiche_conn.trace_id()),
                                )));
                            }

                            return Poll::Pending;
                        }
                        Err(err) => {
                            return {
                                if state.quiche_conn.is_closed() {
                                    handle_stream_close(state, stream_id);
                                }

                                Poll::Ready(Err(into_io_error(err)))
                            }
                        }
                    }
                },
            )
            .await
    }

    /// Create new future for recv stream data
    pub async fn stream_recv<'a>(
        &self,
        stream_id: u64,
        buf: &'a mut [u8],
    ) -> io::Result<(usize, bool)> {
        self.state
            .on_fn(
                QuicConnEvents::StreamRecv(self.trace_id.to_string(), stream_id),
                |state, _| {
                    if state.quiche_conn.is_closed() {
                        handle_stream_close(state, stream_id);
                        return Poll::Ready(Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            format!("conn={} closed", state.quiche_conn.trace_id()),
                        )));
                    }

                    match state.quiche_conn.stream_recv(stream_id, buf) {
                        Ok(recv_size) => {
                            state.notify(QuicConnEvents::Recv(self.trace_id.to_string()));

                            if state.quiche_conn.is_closed() {
                                handle_stream_close(state, stream_id);
                            }

                            return Poll::Ready(Ok(recv_size));
                        }
                        Err(quiche::Error::Done) => {
                            if state.quiche_conn.is_closed() {
                                handle_stream_close(state, stream_id);
                                return Poll::Ready(Err(io::Error::new(
                                    io::ErrorKind::BrokenPipe,
                                    format!("conn={} closed", state.quiche_conn.trace_id()),
                                )));
                            }

                            log::trace!(
                                "{:?} Pending",
                                QuicConnEvents::StreamRecv(self.trace_id.to_string(), stream_id)
                            );

                            return Poll::Pending;
                        }
                        Err(err) => {
                            if state.quiche_conn.is_closed() {
                                handle_stream_close(state, stream_id);
                            }
                            return Poll::Ready(Err(into_io_error(err)));
                        }
                    }
                },
            )
            .await
    }

    /// Open new stream to communicate with remote peer.
    pub fn open_stream(&self) -> QuicStream {
        let id = self.stream_id_seed.fetch_add(2, Ordering::SeqCst);

        QuicStream::new(id, self.clone())
    }

    pub fn close_stream(&self, stream_id: u64) {
        // pin lock like
        loop {
            if self
                .state
                .try_lock_with(|state| {
                    state.opened_streams.remove(&stream_id);
                })
                .is_none()
            {
                continue;
            }

            break;
        }
    }

    pub(super) async fn is_stream_closed(&self, stream_id: u64) -> bool {
        self.state
            .with(|mediator| mediator.quiche_conn.stream_finished(stream_id))
    }

    /// Close connection.
    pub(super) async fn close(&self, app: bool, err: u64, reason: &[u8]) -> io::Result<()> {
        self.state.with_mut(|state| {
            state
                .quiche_conn
                .close(app, err, reason)
                .map_err(into_io_error)
        })
    }

    pub(super) async fn is_closed(&self) -> bool {
        self.state.with(|state| state.quiche_conn.is_closed())
    }

    pub async fn accept(&self) -> Option<QuicStream> {
        self.state
            .on_fn(
                QuicConnEvents::Accept(self.trace_id.to_string()),
                |state, _| {
                    if state.quiche_conn.is_closed() {
                        return Poll::Ready(None);
                    }

                    if let Some(stream_id) = state.incoming_streams.pop_front() {
                        return Poll::Ready(Some(QuicStream::new(stream_id, self.clone())));
                    } else {
                        return Poll::Pending;
                    }
                },
            )
            .await
    }
}
