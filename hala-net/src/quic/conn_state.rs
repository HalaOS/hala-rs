use std::{
    collections::{HashSet, VecDeque},
    fmt::Debug,
    io,
    rc::Rc,
    time::{Duration, Instant},
};

use event_map::{
    locks::{Locker, WaitableLocker, WaitableSpinMutex},
    EventMap, Reason,
};

use hala_io_util::{get_local_poller, local_io_spawn, timeout_with};
use quiche::{ConnectionId, RecvInfo, SendInfo};

use crate::{errors::into_io_error, quic::QuicStream};

/// Quic connection state object
pub struct RawQuicConnState {
    stream_id_seed: u64,
    /// quiche connection instance.
    quiche_conn: quiche::Connection,
    /// Opened stream id set
    opened_streams: HashSet<u64>,
    /// Incoming stream deque.
    incoming_streams: VecDeque<u64>,
    /// Send ping package flag.
    send_ack_eliciting_instant: Instant,
}

impl Debug for RawQuicConnState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuicConnState")
            .field("stream_id_seed", &self.stream_id_seed)
            .field("opened_streams", &self.opened_streams.len())
            .field("incoming_streams", &self.incoming_streams.len())
            .finish()
    }
}

impl RawQuicConnState {
    /// Create new `QuicConnState` from [`Connection`](quiche::Connection)
    pub fn new(quiche_conn: quiche::Connection, stream_id_seed: u64) -> Self {
        Self {
            stream_id_seed,
            quiche_conn,
            opened_streams: Default::default(),
            incoming_streams: Default::default(),
            send_ack_eliciting_instant: Instant::now(),
        }
    }

    fn is_closed_or_draining(&self) -> bool {
        self.quiche_conn.is_closed() || self.quiche_conn.is_draining()
    }
}

impl Drop for RawQuicConnState {
    fn drop(&mut self) {
        log::trace!("dropping conn={}", self.quiche_conn.trace_id());
    }
}

/// `QuicConnState` support event variant.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum QuicConnEvents {
    Send(ConnectionId<'static>),
    Recv(ConnectionId<'static>),
    StreamSend(ConnectionId<'static>, u64),
    StreamRecv(ConnectionId<'static>, u64),
    Accept(ConnectionId<'static>),
    OpenStream,
}

async fn handle_accept(
    mediator: &EventMap<QuicConnEvents>,
    state: &mut RawQuicConnState,
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

        mediator.notify_one(
            &QuicConnEvents::Accept(state.quiche_conn.source_id().into_owned()),
            Reason::On,
        );
    }
}

async fn handle_stream(mediator: &EventMap<QuicConnEvents>, state: &mut RawQuicConnState) {
    let mut events = vec![];

    for stream_id in state.quiche_conn.readable() {
        handle_accept(mediator, state, stream_id).await;

        events.push(QuicConnEvents::StreamRecv(
            state.quiche_conn.source_id().into_owned(),
            stream_id,
        ));
    }

    mediator.notify_all(&events, Reason::On);

    for stream_id in state.quiche_conn.writable() {
        handle_accept(mediator, state, stream_id).await;

        events.push(QuicConnEvents::StreamSend(
            state.quiche_conn.source_id().into_owned(),
            stream_id,
        ));
    }

    mediator.notify_all(&events, Reason::On);
}

fn handle_close(mediator: &EventMap<QuicConnEvents>) {
    mediator.notify_any(Reason::Cancel);
}

/// Quic connection state object
#[derive(Clone)]
pub struct QuicConnState {
    conn_state: Rc<WaitableSpinMutex<RawQuicConnState>>,
    /// core inner state.
    pub(crate) mediator: EventMap<QuicConnEvents>,
    /// String type trace id.
    pub conn_id: Rc<ConnectionId<'static>>,
}

impl QuicConnState {
    pub fn new(quiche_conn: quiche::Connection, stream_id_seed: u64) -> Self {
        Self {
            conn_id: Rc::new(quiche_conn.source_id().into_owned()),
            conn_state: Rc::new(WaitableSpinMutex::new(RawQuicConnState::new(
                quiche_conn,
                stream_id_seed,
            ))),
            mediator: EventMap::default(),
        }
    }

    /// Create new future for send connection data
    pub async fn send<'a>(&self, buf: &'a mut [u8]) -> io::Result<(usize, SendInfo)> {
        let event = QuicConnEvents::Send((*self.conn_id).clone());

        log::trace!("{:?} try get conn_state locker", event);

        let mut conn_state = self.conn_state.async_lock().await;

        log::trace!("{:?} locked conn_state", event);

        loop {
            if conn_state.is_closed_or_draining() {
                handle_close(&self.mediator);

                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!("quic conn closed, id={}", conn_state.quiche_conn.trace_id()),
                ));
            }

            match conn_state.quiche_conn.send(buf) {
                Ok((send_size, send_info)) => {
                    self.mediator
                        .notify_one(&QuicConnEvents::Recv((*self.conn_id).clone()), Reason::On);

                    handle_stream(&self.mediator, &mut conn_state).await;

                    log::trace!(
                        "conn={:?}, send_size={:?}, send_info={:?}",
                        conn_state.quiche_conn.trace_id(),
                        send_size,
                        send_info
                    );

                    return Ok((send_size, send_info));
                }
                Err(quiche::Error::Done) => {
                    log::trace!("conn={:?} send done", conn_state.quiche_conn.trace_id());

                    if conn_state.is_closed_or_draining() {
                        handle_close(&self.mediator);

                        return Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            format!("conn={} closed", conn_state.quiche_conn.trace_id()),
                        ));
                    }

                    let timer_timeout = conn_state.quiche_conn.timeout();

                    let result = timeout_with(
                        async {
                            self.mediator
                                .wait(event.clone(), conn_state)
                                .await
                                .map_err(into_io_error)
                        },
                        timer_timeout,
                        get_local_poller()?,
                    )
                    .await;

                    match result {
                        Ok(guard) => {
                            conn_state = guard;

                            log::trace!("{:?} wakeup", event);

                            continue;
                        }
                        Err(err) if err.kind() == io::ErrorKind::TimedOut => {
                            self.mediator.wait_cancel(&event);

                            conn_state = self.conn_state.async_lock().await;

                            if conn_state.send_ack_eliciting_instant.elapsed()
                                > Duration::from_millis(200)
                            {
                                conn_state
                                    .quiche_conn
                                    .send_ack_eliciting()
                                    .map_err(into_io_error)?;

                                conn_state.send_ack_eliciting_instant = Instant::now();

                                log::trace!(
                                    "conn={:?} send ack_eliciting",
                                    conn_state.quiche_conn.trace_id()
                                );

                                continue;
                            }

                            conn_state.quiche_conn.on_timeout();

                            log::trace!("{:?} on_timeout", event);

                            continue;
                        }
                        Err(err) => {
                            self.mediator.wait_cancel(&event);

                            return Err(into_io_error(err));
                        }
                    }
                }
                Err(err) => {
                    if conn_state.is_closed_or_draining() {
                        handle_close(&self.mediator);
                    }

                    return Err(into_io_error(err));
                }
            }
        }
    }

    /// Create new future for recv connection data
    pub async fn recv<'a>(&self, buf: &'a mut [u8], recv_info: RecvInfo) -> io::Result<usize> {
        let event = QuicConnEvents::Recv((*self.conn_id).clone());

        let mut conn_state = self.conn_state.async_lock().await;

        loop {
            if conn_state.is_closed_or_draining() {
                handle_close(&self.mediator);

                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!("quic conn closed, id={}", conn_state.quiche_conn.trace_id()),
                ));
            }

            match conn_state.quiche_conn.recv(buf, recv_info.clone()) {
                Ok(recv_size) => {
                    self.mediator
                        .notify_one(&QuicConnEvents::Send((*self.conn_id).clone()), Reason::On);

                    handle_stream(&self.mediator, &mut conn_state).await;

                    return Ok(recv_size);
                }
                Err(quiche::Error::Done) => {
                    log::trace!("{:?} done ", event);

                    if conn_state.is_closed_or_draining() {
                        handle_close(&self.mediator);

                        return Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            format!("conn={} closed", conn_state.quiche_conn.trace_id()),
                        ));
                    }

                    conn_state = self
                        .mediator
                        .wait(event.clone(), conn_state)
                        .await
                        .map_err(into_io_error)?;

                    continue;
                }
                Err(err) => {
                    if conn_state.is_closed_or_draining() {
                        handle_close(&self.mediator);
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
        let event = QuicConnEvents::StreamSend((*self.conn_id).clone(), stream_id);

        let mut conn_state = self.conn_state.async_lock().await;

        loop {
            if conn_state.is_closed_or_draining() {
                handle_close(&self.mediator);

                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!("quic conn closed, id={}", conn_state.quiche_conn.trace_id()),
                ));
            }

            match conn_state.quiche_conn.stream_send(stream_id, buf, fin) {
                Ok(recv_size) => {
                    self.mediator
                        .notify_one(&QuicConnEvents::Send((*self.conn_id).clone()), Reason::On);

                    if fin {
                        log::trace!("{:?}, status=fin", event);
                    }

                    return Ok(recv_size);
                }
                Err(quiche::Error::Done) => {
                    log::trace!("StreamSend({:?}, {}) done ", self.conn_id, stream_id);

                    if conn_state.is_closed_or_draining() {
                        handle_close(&self.mediator);

                        return Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            format!("conn={} closed", conn_state.quiche_conn.trace_id()),
                        ));
                    }

                    conn_state = self
                        .mediator
                        .wait(event.clone(), conn_state)
                        .await
                        .map_err(into_io_error)?;

                    continue;
                }
                Err(err) => {
                    if conn_state.is_closed_or_draining() {
                        handle_close(&self.mediator);
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
        let event = QuicConnEvents::StreamRecv((*self.conn_id).clone(), stream_id);

        let mut conn_state = self.conn_state.async_lock().await;

        loop {
            if conn_state.is_closed_or_draining() {
                handle_close(&self.mediator);

                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!("quic conn closed, id={}", conn_state.quiche_conn.trace_id()),
                ));
            }

            match conn_state.quiche_conn.stream_recv(stream_id, buf) {
                Ok((read_size, fin)) => {
                    self.mediator.notify_all(
                        &[
                            QuicConnEvents::Send((*self.conn_id).clone()),
                            QuicConnEvents::Recv((*self.conn_id).clone()),
                        ],
                        Reason::On,
                    );

                    if fin {
                        log::trace!("{:?}, status=fin", event);
                    }

                    return Ok((read_size, fin));
                }
                Err(quiche::Error::Done) => {
                    log::trace!("StreamSend({:?}, {}) done ", self.conn_id, stream_id);

                    if conn_state.is_closed_or_draining() {
                        handle_close(&self.mediator);

                        return Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            format!("conn={} closed", conn_state.quiche_conn.trace_id()),
                        ));
                    }

                    conn_state = self
                        .mediator
                        .wait(event.clone(), conn_state)
                        .await
                        .map_err(into_io_error)?;

                    continue;
                }
                Err(err) => {
                    if conn_state.is_closed_or_draining() {
                        handle_close(&self.mediator);
                    }

                    return Err(into_io_error(err));
                }
            }
        }
    }

    /// Open new stream to communicate with remote peer.
    pub async fn open_stream(&self) -> io::Result<QuicStream> {
        let mut state = self.conn_state.async_lock().await;

        if state.quiche_conn.is_closed() {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                format!("quic conn closed, id={}", state.quiche_conn.trace_id()),
            ));
        }

        let stream_id = state.stream_id_seed;
        state.stream_id_seed += 4;

        self.mediator
            .notify_one(QuicConnEvents::Send((*self.conn_id).clone()), Reason::On);

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
            .async_lock()
            .await
            .quiche_conn
            .stream_finished(stream_id)
    }

    /// Close connection.
    pub(super) async fn close(&self, app: bool, err: u64, reason: &[u8]) -> io::Result<()> {
        self.conn_state
            .async_lock()
            .await
            .quiche_conn
            .close(app, err, reason)
            .map_err(into_io_error)
    }

    pub(super) async fn is_closed(&self) -> bool {
        self.conn_state.async_lock().await.quiche_conn.is_closed()
    }

    pub async fn accept(&self) -> Option<QuicStream> {
        let event = QuicConnEvents::Accept((*self.conn_id).clone());

        let mut conn_state = self.conn_state.async_lock().await;

        loop {
            if conn_state.is_closed_or_draining() {
                log::trace!("{:?}, conn_status=closed", event);
                return None;
            }

            if let Some(stream_id) = conn_state.incoming_streams.pop_front() {
                return Some(QuicStream::new(stream_id, self.clone()));
            } else {
                conn_state = match self.mediator.wait(&event, conn_state).await {
                    Err(_) => return None,
                    Ok(conn_state) => conn_state,
                };
            }
        }
    }
}
