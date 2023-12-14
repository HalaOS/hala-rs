use std::{
    collections::{HashMap, HashSet, VecDeque},
    io,
    sync::Arc,
    task::{Poll, Waker},
};

use futures::{lock::Mutex, Future, FutureExt};
use quiche::{RecvInfo, SendInfo};

use crate::errors::into_io_error;

use super::QuicStream;

struct RawConnState {
    /// The stream id seed
    stream_id_seed: u64,
    /// quiche connection instance.
    quiche_conn: quiche::Connection,
    /// Waker for connection send operation.
    send_waker: Option<Waker>,
    /// Waker for connection recv operation.
    recv_waker: Option<Waker>,
    /// Accept stream waker
    accept_waker: Option<Waker>,
    /// Wakers for stream send
    stream_send_wakers: HashMap<u64, Waker>,
    /// Wakers for stream recv
    stream_recv_wakers: HashMap<u64, Waker>,
    /// Opened stream id set
    opened_streams: HashSet<u64>,
    /// Incoming stream deque.
    incoming_streams: VecDeque<QuicStream>,
}

impl RawConnState {
    fn new(quiche_conn: quiche::Connection, stream_id_seed: u64) -> Self {
        RawConnState {
            stream_id_seed,
            quiche_conn,
            send_waker: Default::default(),
            recv_waker: Default::default(),
            accept_waker: Default::default(),
            stream_send_wakers: Default::default(),
            stream_recv_wakers: Default::default(),
            opened_streams: Default::default(),
            incoming_streams: Default::default(),
        }
    }
    fn try_stream_wake(&mut self) {
        for stream_id in self.quiche_conn.readable() {
            if let Some(waker) = self.stream_recv_wakers.remove(&stream_id) {
                waker.wake();
            }
        }

        for stream_id in self.quiche_conn.writable() {
            if let Some(waker) = self.stream_send_wakers.remove(&stream_id) {
                waker.wake();
            }
        }
    }

    fn try_conn_send_wake(&mut self) {
        if let Some(waker) = self.send_waker.take() {
            waker.wake();
        }
    }

    fn try_conn_recv_wake(&mut self) {
        if let Some(waker) = self.recv_waker.take() {
            waker.wake();
        }
    }
}

/// Quic connection instance created by `connect` / `accept` methods.
#[derive(Debug, Clone)]
pub struct QuicConnState {
    /// Mutex protected quiche connection instance.
    state: Arc<Mutex<RawConnState>>,
}

impl QuicConnState {
    pub fn new(quiche_conn: quiche::Connection, stream_id_seed: u64) -> Self {
        Self {
            state: Arc::new(Mutex::new(RawConnState::new(quiche_conn, stream_id_seed))),
        }
    }
    /// Create new future for send connection data
    pub fn send<'a>(&self, buf: &'a mut [u8]) -> QuicConnSend<'a> {
        QuicConnSend {
            buf,
            state: self.state.clone(),
        }
    }

    /// Create new future for recv connection data
    pub fn recv<'a>(&self, buf: &'a mut [u8], recv_info: RecvInfo) -> QuicConnRecv<'a> {
        QuicConnRecv {
            buf,
            recv_info,
            state: self.state.clone(),
        }
    }

    /// Create new future for send stream data
    pub(super) fn stream_send<'a>(
        &self,
        stream_id: u64,
        buf: &'a [u8],
        fin: bool,
    ) -> QuicStreamSend<'a> {
        QuicStreamSend {
            buf,
            stream_id,
            fin,
            state: self.state.clone(),
        }
    }

    /// Create new future for recv stream data
    pub(super) fn stream_recv<'a>(&self, stream_id: u64, buf: &'a mut [u8]) -> QuicStreamRecv<'a> {
        QuicStreamRecv {
            buf,
            stream_id,
            state: self.state.clone(),
        }
    }

    /// Open new stream to communicate with remote peer.
    pub async fn open_stream(&self) -> QuicStream {
        let mut state = self.state.lock().await;

        let stream_id = state.stream_id_seed;

        state.stream_id_seed += 2;

        state.opened_streams.insert(stream_id);

        QuicStream::new(stream_id, self.clone())
    }

    pub async fn accept(&self) -> QuicConnAccept {
        QuicConnAccept {
            state: self.state.clone(),
        }
    }
}

/// Future created by [`send`](QuicInnerConn::send) method
pub struct QuicConnSend<'a> {
    buf: &'a mut [u8],
    state: Arc<Mutex<RawConnState>>,
}

impl<'a> Future for QuicConnSend<'a> {
    type Output = io::Result<(usize, SendInfo)>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let state = self.state.clone();
        let mut state = match state.lock().poll_unpin(cx) {
            Poll::Ready(guard) => guard,
            _ => return Poll::Pending,
        };

        match state.quiche_conn.send(self.buf) {
            Ok((send_size, send_info)) => {
                state.try_stream_wake();

                return Poll::Ready(Ok((send_size, send_info)));
            }
            Err(quiche::Error::Done) => {
                state.send_waker = Some(cx.waker().clone());
                return Poll::Pending;
            }
            Err(err) => return Poll::Ready(Err(into_io_error(err))),
        }
    }
}

/// Future created by [`recv`](QuicInnerConn::recv) method
pub struct QuicConnRecv<'a> {
    buf: &'a mut [u8],
    recv_info: RecvInfo,
    state: Arc<Mutex<RawConnState>>,
}

impl<'a> Future for QuicConnRecv<'a> {
    type Output = io::Result<usize>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let state = self.state.clone();
        let mut state = match state.lock().poll_unpin(cx) {
            Poll::Ready(guard) => guard,
            _ => return Poll::Pending,
        };

        let recv_info = self.recv_info;

        match state.quiche_conn.recv(self.buf, recv_info) {
            Ok(recv_size) => {
                state.try_stream_wake();

                return Poll::Ready(Ok(recv_size));
            }
            Err(quiche::Error::Done) => {
                state.recv_waker = Some(cx.waker().clone());
                return Poll::Pending;
            }
            Err(err) => return Poll::Ready(Err(into_io_error(err))),
        }
    }
}

/// Future created by [`stream_send`](QuicInnerConn::stream_send) method
pub struct QuicStreamSend<'a> {
    buf: &'a [u8],
    stream_id: u64,
    fin: bool,
    state: Arc<Mutex<RawConnState>>,
}

impl<'a> Future for QuicStreamSend<'a> {
    type Output = io::Result<usize>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let state = self.state.clone();
        let mut state = match state.lock().poll_unpin(cx) {
            Poll::Ready(guard) => guard,
            _ => return Poll::Pending,
        };

        match state
            .quiche_conn
            .stream_send(self.stream_id, self.buf, self.fin)
        {
            Ok(recv_size) => {
                state.try_conn_send_wake();

                return Poll::Ready(Ok(recv_size));
            }
            Err(quiche::Error::Done) => {
                state
                    .stream_send_wakers
                    .insert(self.stream_id, cx.waker().clone());
                return Poll::Pending;
            }
            Err(err) => return Poll::Ready(Err(into_io_error(err))),
        }
    }
}

/// Future created by [`stream_recv`](QuicInnerConn::stream_recv) method
pub struct QuicStreamRecv<'a> {
    buf: &'a mut [u8],
    stream_id: u64,
    state: Arc<Mutex<RawConnState>>,
}

impl<'a> Future for QuicStreamRecv<'a> {
    type Output = io::Result<(usize, bool)>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let state = self.state.clone();
        let mut state = match state.lock().poll_unpin(cx) {
            Poll::Ready(guard) => guard,
            _ => return Poll::Pending,
        };

        match state.quiche_conn.stream_recv(self.stream_id, self.buf) {
            Ok(recv_size) => {
                state.try_conn_recv_wake();

                return Poll::Ready(Ok(recv_size));
            }
            Err(quiche::Error::Done) => {
                state
                    .stream_recv_wakers
                    .insert(self.stream_id, cx.waker().clone());
                return Poll::Pending;
            }
            Err(err) => return Poll::Ready(Err(into_io_error(err))),
        }
    }
}

/// Future created by [`accept`](QuicInnerConn::accept) method
pub struct QuicConnAccept {
    state: Arc<Mutex<RawConnState>>,
}

impl Future for QuicConnAccept {
    type Output = io::Result<QuicStream>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let state = self.state.clone();
        let mut state = match state.lock().poll_unpin(cx) {
            Poll::Ready(guard) => guard,
            _ => return Poll::Pending,
        };

        if let Some(stream) = state.incoming_streams.pop_front() {
            return Poll::Ready(Ok(stream));
        } else {
            state.accept_waker = Some(cx.waker().clone());

            return Poll::Pending;
        }
    }
}
