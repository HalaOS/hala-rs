use std::{
    collections::{HashSet, VecDeque},
    io,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    task::Poll,
};

use future_mediator::Mediator;
use futures::FutureExt;
use hala_io_util::Sleep;
use quiche::{RecvInfo, SendInfo};

use crate::{errors::into_io_error, quic::QuicStream};

/// Quic connection inner state object
struct RawConnState {
    /// quiche connection instance.
    quiche_conn: quiche::Connection,

    /// Opened stream id set
    opened_streams: HashSet<u64>,
    /// Incoming stream deque.
    incoming_streams: VecDeque<u64>,
}

impl RawConnState {
    fn new(quiche_conn: quiche::Connection) -> Self {
        Self {
            quiche_conn,
            opened_streams: Default::default(),
            incoming_streams: Default::default(),
        }
    }
}

impl Drop for RawConnState {
    fn drop(&mut self) {
        log::trace!("dropping conn={}", self.quiche_conn.trace_id());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Ops {
    Send,
    Recv,
    StreanSend,
    StreamRecv,
    Accept,
}

/// Quic connection state object
#[derive(Clone)]
pub(crate) struct QuicConnState {
    /// core inner state.
    state: Mediator<RawConnState, Ops>,

    /// stream id generator seed
    stream_id_seed: Arc<AtomicU64>,

    pub(crate) trace_id: Arc<String>,
}

impl QuicConnState {
    pub(crate) fn new(quiche_conn: quiche::Connection, stream_id_seed: u64) -> Self {
        Self {
            trace_id: Arc::new(quiche_conn.trace_id().to_owned()),
            state: Mediator::new(RawConnState::new(quiche_conn)),
            stream_id_seed: Arc::new(stream_id_seed.into()),
        }
    }

    /// Create new future for send connection data
    pub(crate) async fn send<'a>(&self, buf: &'a mut [u8]) -> io::Result<(usize, SendInfo)> {
        let mut sleep: Option<Sleep> = None;

        self.state
            .on_fn(Ops::Send, |state, cx| {
                if state.quiche_conn.is_closed() {
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

                            state.notify_all(&[Ops::StreamRecv, Ops::StreanSend]);

                            return Poll::Ready(Ok((send_size, send_info)));
                        }
                        Err(quiche::Error::Done) => {
                            if let Some(expired) = state.quiche_conn.timeout() {
                                log::trace!(
                                    "QuicConnSend(conn_id={}) add timeout({:?})",
                                    state.quiche_conn.trace_id(),
                                    expired
                                );

                                let mut timeout = Sleep::new(expired)?;

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
                                state.notify_all(&[
                                    Ops::Accept,
                                    Ops::Recv,
                                    Ops::StreamRecv,
                                    Ops::StreanSend,
                                ]);
                            }

                            return Poll::Pending;
                        }
                        Err(err) => return Poll::Ready(Err(into_io_error(err))),
                    }
                }
            })
            .await
    }

    /// Create new future for recv connection data
    pub(crate) async fn recv<'a>(
        &self,
        buf: &'a mut [u8],
        recv_info: RecvInfo,
    ) -> io::Result<usize> {
        self.state
            .on_fn(Ops::Recv, |state, _| {
                if state.quiche_conn.is_closed() {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        format!("conn={} closed", state.quiche_conn.trace_id()),
                    )));
                }

                let recv_info = recv_info;

                match state.quiche_conn.recv(buf, recv_info) {
                    Ok(recv_size) => {
                        state.notify_all(&[Ops::StreamRecv, Ops::StreanSend]);

                        if state.quiche_conn.is_closed() {
                            state.notify(Ops::Accept);
                        }

                        return Poll::Ready(Ok(recv_size));
                    }
                    Err(quiche::Error::Done) => {
                        state.notify(Ops::Recv);
                        return Poll::Pending;
                    }
                    Err(err) => {
                        state.notify(Ops::Accept);
                        return Poll::Ready(Err(into_io_error(err)));
                    }
                }
            })
            .await
    }

    /// Create new future for send stream data
    pub(crate) async fn stream_send<'a>(
        &self,
        stream_id: u64,
        buf: &'a [u8],
        fin: bool,
    ) -> io::Result<usize> {
        self.state
            .on_fn(Ops::StreanSend, |state, _| {
                if state.quiche_conn.is_closed() {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        format!("conn={} closed", state.quiche_conn.trace_id()),
                    )));
                }

                match state.quiche_conn.stream_send(stream_id, buf, fin) {
                    Ok(recv_size) => {
                        state.notify(Ops::Send);

                        return Poll::Ready(Ok(recv_size));
                    }
                    Err(quiche::Error::Done) => {
                        return Poll::Pending;
                    }
                    Err(err) => return Poll::Ready(Err(into_io_error(err))),
                }
            })
            .await
    }

    /// Create new future for recv stream data
    pub(crate) async fn stream_recv<'a>(
        &self,
        stream_id: u64,
        buf: &'a mut [u8],
    ) -> io::Result<(usize, bool)> {
        self.state
            .on_fn(Ops::StreamRecv, |state, _| {
                if state.quiche_conn.is_closed() {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        format!("conn={} closed", state.quiche_conn.trace_id()),
                    )));
                }

                match state.quiche_conn.stream_recv(stream_id, buf) {
                    Ok(recv_size) => {
                        state.notify(Ops::Recv);

                        return Poll::Ready(Ok(recv_size));
                    }
                    Err(quiche::Error::Done) => {
                        return Poll::Pending;
                    }
                    Err(err) => return Poll::Ready(Err(into_io_error(err))),
                }
            })
            .await
    }

    /// Open new stream to communicate with remote peer.
    pub(crate) fn open_stream(&self) -> QuicStream {
        let id = self.stream_id_seed.fetch_add(2, Ordering::SeqCst);

        QuicStream::new(id, self.clone())
    }

    pub(crate) fn close_stream(&self, stream_id: u64) {
        // pin lock like
        loop {
            let state = self.state.try_lock();

            if state.is_none() {
                continue;
            }

            let mut state = state.unwrap();

            state.opened_streams.remove(&stream_id);

            break;
        }
    }

    pub(super) async fn is_stream_closed(&self, stream_id: u64) -> bool {
        self.state
            .with(|mediator| mediator.quiche_conn.stream_finished(stream_id))
            .await
    }

    /// Close connection.
    pub(super) async fn close(&self, app: bool, err: u64, reason: &[u8]) -> io::Result<()> {
        self.state
            .with_mut(|state| {
                state
                    .quiche_conn
                    .close(app, err, reason)
                    .map_err(into_io_error)
            })
            .await
    }

    pub(super) async fn is_closed(&self) -> bool {
        self.state.with(|state| state.quiche_conn.is_closed()).await
    }

    pub(crate) async fn accept(&self) -> Option<QuicStream> {
        self.state
            .on_fn(Ops::Accept, |state, _| {
                if state.quiche_conn.is_closed() {
                    return Poll::Ready(None);
                }

                if let Some(stream_id) = state.incoming_streams.pop_front() {
                    return Poll::Ready(Some(QuicStream::new(stream_id, self.clone())));
                } else {
                    return Poll::Pending;
                }
            })
            .await
    }
}
