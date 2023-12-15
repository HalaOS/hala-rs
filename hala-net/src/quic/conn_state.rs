use std::{
    collections::{HashMap, HashSet, VecDeque},
    io,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    task::{Poll, Waker},
};

use futures::{lock::Mutex, Future, FutureExt};
use quiche::{RecvInfo, SendInfo};

use crate::{errors::into_io_error, quic::QuicStream};

/// Quic connection inner state object
struct RawConnState {
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
    incoming_streams: VecDeque<u64>,
}

impl RawConnState {
    fn new(quiche_conn: quiche::Connection) -> Self {
        Self {
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

    fn wakeup_stream(&mut self) {
        for stream_id in self.quiche_conn.readable() {
            self.wakeup_accept(stream_id);

            if let Some(waker) = self.stream_recv_wakers.remove(&stream_id) {
                waker.wake();
            }
        }

        for stream_id in self.quiche_conn.writable() {
            self.wakeup_accept(stream_id);

            if let Some(waker) = self.stream_send_wakers.remove(&stream_id) {
                waker.wake();
            }
        }
    }

    fn wakeup_accept(&mut self, stream_id: u64) {
        if !self.opened_streams.contains(&stream_id) {
            self.opened_streams.insert(stream_id);
            self.incoming_streams.push_back(stream_id);

            if let Some(waker) = self.accept_waker.take() {
                waker.wake();
            }
        }
    }

    fn wakeup_conn_send(&mut self) {
        if let Some(waker) = self.send_waker.take() {
            waker.wake();
        }
    }

    fn wakeup_conn_recv(&mut self) {
        if let Some(waker) = self.recv_waker.take() {
            waker.wake();
        }
    }
}

/// Quic connection state object
#[derive(Debug, Clone)]
pub(crate) struct QuicConnState {
    /// core inner state.
    state: Arc<Mutex<RawConnState>>,

    /// stream id generator seed
    stream_id_seed: Arc<AtomicU64>,

    pub(crate) trace_id: Arc<String>,
}

impl QuicConnState {
    pub(crate) fn new(quiche_conn: quiche::Connection, stream_id_seed: u64) -> Self {
        Self {
            trace_id: Arc::new(quiche_conn.trace_id().to_owned()),
            state: Arc::new(Mutex::new(RawConnState::new(quiche_conn))),
            stream_id_seed: Arc::new(stream_id_seed.into()),
        }
    }

    /// Create new future for send connection data
    pub(crate) fn send<'a>(&self, buf: &'a mut [u8]) -> QuicConnSend<'a> {
        QuicConnSend {
            buf,
            state: self.state.clone(),
        }
    }

    /// Create new future for recv connection data
    pub(crate) fn recv<'a>(&self, buf: &'a mut [u8], recv_info: RecvInfo) -> QuicConnRecv<'a> {
        QuicConnRecv {
            buf,
            recv_info,
            state: self.state.clone(),
        }
    }

    /// Create new future for send stream data
    pub(crate) fn stream_send<'a>(
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
    pub(crate) fn stream_recv<'a>(&self, stream_id: u64, buf: &'a mut [u8]) -> QuicStreamRecv<'a> {
        QuicStreamRecv {
            buf,
            stream_id,
            state: self.state.clone(),
        }
    }

    /// Open new stream to communicate with remote peer.
    pub(crate) fn open_stream(&self) -> QuicStream {
        let id = self.stream_id_seed.fetch_add(2, Ordering::SeqCst);

        QuicStream::new(id, self.clone())
    }

    pub(crate) fn close_stream(&self, stream_id: u64) {
        // no future lock state in pending state
        loop {
            let state = self.state.try_lock();

            if state.is_none() {
                continue;
            }

            let mut state = state.unwrap();

            state.opened_streams.remove(&stream_id);
        }
    }

    pub(crate) fn accept(&self) -> QuicConnAccept {
        QuicConnAccept {
            state: self.clone(),
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
                state.wakeup_stream();

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
                state.wakeup_stream();

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
                state.wakeup_conn_send();

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
                state.wakeup_conn_recv();

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
    state: QuicConnState,
}

impl Future for QuicConnAccept {
    type Output = io::Result<QuicStream>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let state = self.state.clone();
        let mut state = match state.state.lock().poll_unpin(cx) {
            Poll::Ready(guard) => guard,
            _ => return Poll::Pending,
        };

        if let Some(stream_id) = state.incoming_streams.pop_front() {
            return Poll::Ready(Ok(QuicStream::new(stream_id, self.state.clone())));
        } else {
            state.accept_waker = Some(cx.waker().clone());

            return Poll::Pending;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use futures::lock::Mutex;

    use hala_io_util::io_spawn;

    #[hala_io_test::test]
    async fn test_future_lock() {
        let a = Arc::new(Mutex::new(1));

        let a_cloned = a.clone();

        io_spawn(async move {
            *a_cloned.lock().await = 2;

            Ok(())
        })
        .unwrap();

        loop {
            let state = a.try_lock();

            if state.is_none() {
                continue;
            }

            break;
        }
    }
}
