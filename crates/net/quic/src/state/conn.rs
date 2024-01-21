//! The quic connection state matchine implementation.

use std::{
    collections::{HashSet, VecDeque},
    fmt::Debug,
    io, mem,
    ops::{self, DerefMut},
    sync::Arc,
    time::{Duration, Instant},
};

use hala_future::{
    event_map::{self, EventMap},
    executor::future_spawn,
};
use hala_io::timeout;
use hala_ops::deref::DerefExt;
use hala_sync::*;
use quiche::{ConnectionId, RecvInfo, SendInfo};

use crate::errors::into_io_error;

/// The io event variants for quic connection state mache.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum QuicConnStateEvent {
    /// This event notify listener that this state machine is now readable.
    Readable(ConnectionId<'static>),

    /// This event notify listener that this state machine is now writable.
    Writable(ConnectionId<'static>),

    /// This event notify listener that one stream of this state machine is now readable.
    StreamReadable(ConnectionId<'static>, u64),

    /// This event notify listener that one stream of this state machine is now writable.
    StreamWritable(ConnectionId<'static>, u64),

    /// This event notify listener that one incoming stream is valid.
    Accept(ConnectionId<'static>),
}

struct RawQuicConnState {
    /// quiche connection state machine.
    quiche_conn: quiche::Connection,
    /// The timestamp of last ping packet sent.
    send_ack_eliciting_instant: Instant,
    /// The interval between two ping packets.
    ping_timeout: Duration,
    /// When a connection sees a stream ID for the first time,
    /// it is placed into this stream ID set.
    register_incoming_stream_ids: HashSet<u64>,
    /// When a new outgoing stream is created, it's id is placed into this set.
    /// The state machine use this set to avoid recv error before first data send.
    register_outgoing_stream_ids: HashSet<u64>,
    /// The latest outgoing stream id on record.
    lastest_outgoing_stream_id: u64,
    /// Incoming stream id buffer.
    incoming: VecDeque<u64>,
    /// The queue of stream ids for dropping stream.
    dropping_stream_queue: VecDeque<u64>,
    /// The collection of half closed stream.
    half_closed_streams: VecDeque<HalfClosedStream>,
}

#[derive(Clone, Copy)]
struct HalfClosedStream {
    stream_id: u64,
    start_instant: Instant,
}

impl From<u64> for HalfClosedStream {
    fn from(value: u64) -> Self {
        Self::new(value)
    }
}

impl Debug for HalfClosedStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "half_close_stream={:}, elapsed={:?}",
            self.stream_id,
            self.start_instant.elapsed()
        )
    }
}

impl HalfClosedStream {
    fn new(stream_id: u64) -> Self {
        Self {
            stream_id,
            start_instant: Instant::now(),
        }
    }

    fn is_timeout(&self, timeout: Duration) -> bool {
        self.start_instant.elapsed() >= timeout
    }
}

impl RawQuicConnState {
    fn new(
        quiche_conn: quiche::Connection,
        ping_timeout: Duration,
        first_outgoing_stream_id: u64,
    ) -> Self {
        let mut this = Self {
            quiche_conn,
            ping_timeout,
            send_ack_eliciting_instant: Instant::now(),
            register_incoming_stream_ids: Default::default(),
            register_outgoing_stream_ids: Default::default(),
            lastest_outgoing_stream_id: first_outgoing_stream_id,
            incoming: Default::default(),
            dropping_stream_queue: Default::default(),
            half_closed_streams: Default::default(),
        };

        // process initial incoming stream.
        for id in this.quiche_conn.readable() {
            if id % 2 != first_outgoing_stream_id % 2 {
                this.register_incoming_stream_ids.insert(id);
            }

            this.incoming.push_back(id);
        }

        this
    }
}

/// The state matchine for quic connection.
#[derive(Clone)]
pub struct QuicConnState {
    /// The [`QuicConnState`] instance with lock protected.
    state: Arc<AsyncSpinMutex<RawQuicConnState>>,
    /// The [`EventMap`] instance.
    mediator: Arc<EventMap<QuicConnStateEvent>>,
    /// The source id of this connection.
    pub scid: ConnectionId<'static>,
    /// The destination id of this connection.
    pub dcid: ConnectionId<'static>,
    /// Whether or not this is a server-side connection.
    pub is_server: bool,
}

impl Debug for QuicConnState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "QuicConnState, scid={:?}, dcid={:?}, is_server={}",
            self.scid, self.dcid, self.is_server
        )
    }
}

impl QuicConnState {
    /// Create new `QuicConnState` instance.
    pub fn new(
        quiche_conn: quiche::Connection,
        ping_timeout: Duration,
        first_outgoing_stream_id: u64,
    ) -> Self {
        Self {
            is_server: quiche_conn.is_server(),
            scid: quiche_conn.source_id().into_owned(),
            dcid: quiche_conn.destination_id().into_owned(),
            state: Arc::new(AsyncSpinMutex::new(RawQuicConnState::new(
                quiche_conn,
                ping_timeout,
                first_outgoing_stream_id,
            ))),
            mediator: Arc::new(EventMap::default()),
        }
    }

    fn handle_quic_conn_status<'a, Guard>(&self, state: &mut Guard) -> io::Result<()>
    where
        Guard: DerefMut<Target = RawQuicConnState>,
    {
        if state.quiche_conn.is_closed() || state.quiche_conn.is_draining() {
            self.mediator.notify_any(event_map::Reason::Cancel);

            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                format!("{:?} closed", state.quiche_conn.source_id()),
            ));
        }

        Ok(())
    }

    fn handle_quic_incoming_stream<'a, Guard>(&self, state: &mut Guard, id: u64) -> io::Result<()>
    where
        Guard: DerefMut<Target = RawQuicConnState>,
    {
        // Check the incoming stream id parity and search in register set.
        if state.lastest_outgoing_stream_id % 2 != id % 2
            && !state.register_incoming_stream_ids.contains(&id)
        {
            state.register_incoming_stream_ids.insert(id);
            state.incoming.push_back(id);
            self.mediator.notify_one(
                QuicConnStateEvent::Accept(self.scid.clone()),
                event_map::Reason::On,
            );
        }

        Ok(())
    }

    fn notify_readable<'a, Guard>(&self, state: &mut Guard) -> io::Result<()>
    where
        Guard: DerefMut<Target = RawQuicConnState>,
    {
        self.handle_quic_conn_status(state)?;

        self.mediator.notify_one(
            QuicConnStateEvent::Readable(self.scid.clone()),
            event_map::Reason::On,
        );

        Ok(())
    }

    fn handle_stream_status<'a, Guard>(&self, state: &mut Guard) -> io::Result<()>
    where
        Guard: DerefMut<Target = RawQuicConnState>,
    {
        while let Some(id) = state.quiche_conn.stream_readable_next() {
            self.handle_quic_incoming_stream(state, id)?;
            self.mediator.notify_one(
                QuicConnStateEvent::StreamReadable(self.scid.clone(), id),
                event_map::Reason::On,
            );

            return Ok(());
        }

        while let Some(id) = state.quiche_conn.stream_writable_next() {
            self.mediator.notify_one(
                QuicConnStateEvent::StreamWritable(self.scid.clone(), id),
                event_map::Reason::On,
            );

            return Ok(());
        }

        Ok(())
    }

    fn handle_send_timout<Guard>(&self, state: &mut Guard) -> io::Result<Option<Duration>>
    where
        Guard: DerefMut<Target = RawQuicConnState>,
    {
        let elapsed = state.send_ack_eliciting_instant.elapsed();

        if elapsed > state.ping_timeout {
            state
                .quiche_conn
                .send_ack_eliciting()
                .map_err(into_io_error)?;

            // reset ping timeout
            state.send_ack_eliciting_instant = Instant::now();

            return Ok(None);
        }

        let ping_timout = state.ping_timeout - elapsed;

        if let Some(timeout) = state.quiche_conn.timeout() {
            if timeout < ping_timout {
                if timeout < Duration::from_millis(10) {
                    state.quiche_conn.on_timeout();
                    return Ok(None);
                }

                return Ok(Some(timeout));
            }
        }

        return Ok(Some(ping_timout));
    }

    fn handle_dropping_streams<Guard>(&self, state: &mut Guard) -> io::Result<()>
    where
        Guard: DerefMut<Target = RawQuicConnState>,
    {
        while let Some(stream_id) = state.dropping_stream_queue.pop_front() {
            log::trace!("{:?} handle dropping stream, stream_id={}", self, stream_id);

            match state.quiche_conn.stream_send(stream_id, b"", true) {
                Ok(_) => {
                    log::trace!("{:?} send fin successfully, stream_id={}", self, stream_id);
                }
                Err(err) => {
                    log::trace!(
                        "{:?} send fin failed, stream_id={}, err={}",
                        self,
                        stream_id,
                        err
                    );
                }
            }

            // Warning!!: Stream data reading is prohibited here:
            //
            // The quiche may remove closed stream when invoke stream_recv,
            // this will cause the fin frame to fail to send.

            state.half_closed_streams.push_back(stream_id.into());

            log::trace!("{:?}, stream_id={}, half closed", self, stream_id);
        }

        Ok(())
    }

    fn handle_half_closed_stream_resp<Guard>(
        &self,
        state: &mut Guard,
        stream: HalfClosedStream,
        buf: &mut [u8],
    ) -> bool
    where
        Guard: DerefMut<Target = RawQuicConnState>,
    {
        // using ping timeout as stream half close timeout.
        if stream.is_timeout(state.ping_timeout) {
            _ = state
                .quiche_conn
                .stream_shutdown(stream.stream_id, quiche::Shutdown::Read, 0);

            log::trace!("{:?} {:?}, timeout and force closed", self, stream);

            return true;
        }

        loop {
            match state.quiche_conn.stream_recv(stream.stream_id, buf) {
                Ok((read_size, fin)) => {
                    if !fin {
                        log::trace!("{:?} {:?}, recv_len={}, pending", self, stream, read_size);

                        continue;
                    } else {
                        log::trace!("{:?} {:?}, recv_len={}, closed", self, stream, read_size);

                        return true;
                    }
                }
                Err(quiche::Error::Done) => {
                    log::trace!("{:?} {:?}, pending", self, stream);

                    return false;
                }
                Err(err) => {
                    log::trace!("{:?} {:?}, closed with error, err={}", self, stream, err);

                    return true;
                }
            }
        }
    }

    fn handle_half_closed_streams<Guard>(&self, state: &mut Guard) -> io::Result<()>
    where
        Guard: DerefMut<Target = RawQuicConnState>,
    {
        let mut buf = vec![0; state.quiche_conn.send_quantum()];

        let mut remaining = vec![];

        while let Some(stream_id) = state.half_closed_streams.pop_front() {
            if !self.handle_half_closed_stream_resp(state, stream_id, &mut buf) {
                remaining.push(stream_id);
            }
        }

        for stream_id in remaining {
            state.half_closed_streams.push_back(stream_id);
        }

        Ok(())
    }

    /// ASynchronously read a single QUIC packet to be sent to the peer.
    ///
    /// if there is nothing to read, this function will `pending` until the state changes to
    /// [`writable`](QuicConnStateEvent::Writable).
    pub async fn read(&self, buf: &mut [u8]) -> io::Result<(usize, SendInfo)> {
        let event = QuicConnStateEvent::Readable(self.scid.clone());

        loop {
            log::trace!("{:?} read data", self);

            let mut state = self.state.lock().await;

            self.handle_dropping_streams(&mut state)?;

            match state.quiche_conn.send(buf) {
                Ok((send_size, send_info)) => {
                    log::trace!(
                        "{:?} read data, len={}, elapsed={:?}",
                        self,
                        send_size,
                        send_info.at.elapsed()
                    );

                    // self.handle_stream_status(&mut state)?;

                    return Ok((send_size, send_info));
                }
                Err(quiche::Error::Done) => {
                    let send_timeout = self.handle_send_timout(&mut state)?;

                    if send_timeout.is_none() {
                        // immediately send ping packet.
                        continue;
                    }

                    log::trace!(
                        "{:?} read data pending, timeout={:?}, is_established={}",
                        self,
                        send_timeout,
                        state.quiche_conn.is_established()
                    );

                    let wait_fut = async {
                        self.mediator
                            .wait(event.clone(), state)
                            .await
                            .map_err(into_io_error)
                    };

                    let wait_fut_with_timeout = timeout(wait_fut, send_timeout);

                    match wait_fut_with_timeout.await {
                        Ok(_) => {
                            continue;
                        }
                        Err(err) if err.kind() == io::ErrorKind::TimedOut => {
                            // cancel waiting readable event notify.
                            // self.mediator.wait_cancel(&event);
                            // relock state.
                            let mut state = self.state.lock().await;

                            if state.send_ack_eliciting_instant.elapsed() > state.ping_timeout {
                                state
                                    .quiche_conn
                                    .send_ack_eliciting()
                                    .map_err(into_io_error)?;

                                log::trace!("{:?} ping, timout={:?}", self, state.ping_timeout,);

                                // reset ping timeout
                                state.send_ack_eliciting_instant = Instant::now();

                                continue;
                            }

                            if let Some(timeout_instant) = state.quiche_conn.timeout_instant() {
                                if timeout_instant <= Instant::now() {
                                    state.quiche_conn.on_timeout();

                                    log::debug!(
                                        "{:?} send, timeout={:?}",
                                        self,
                                        state.quiche_conn.timeout()
                                    );
                                }
                            }

                            continue;
                        }
                        Err(err) => {
                            log::error!("{:?} read data pending, err={}", self, err);
                            return Err(into_io_error(err));
                        }
                    }
                }
                Err(err) => {
                    log::error!("{:?} read data, err={}", self, err);

                    self.handle_quic_conn_status(&mut state)?;

                    return Err(into_io_error(err));
                }
            }
        }
    }

    /// Asynchronous write new data to state machine.
    pub async fn write(&self, buf: &mut [u8], recv_info: RecvInfo) -> io::Result<usize> {
        log::trace!("{:?} write data, len={}", self, buf.len());

        let mut state = self.state.lock().await;

        match state.quiche_conn.recv(buf, recv_info) {
            Ok(write_size) => {
                log::trace!("{:?} write data success, len={}", self, write_size);

                self.handle_stream_status(&mut state)?;

                self.handle_half_closed_streams(&mut state)?;

                return Ok(write_size);
            }
            Err(err) => {
                log::trace!("{:?} write data failed, err={}", self, err);

                self.handle_quic_conn_status(&mut state)?;

                return Err(into_io_error(err));
            }
        }
        // }
    }

    /// Writes data to stream.
    pub async fn stream_send(&self, id: u64, buf: &[u8], fin: bool) -> io::Result<usize> {
        let event = QuicConnStateEvent::StreamWritable(self.scid.clone(), id);

        loop {
            log::trace!(
                "{:?} stream write, stream_id={}, len={}, fin={}",
                self,
                id,
                buf.len(),
                fin,
            );
            // Asynchronously lock the [`QuicConnState`]
            let mut state = self.state.lock().await;

            match state.quiche_conn.stream_send(id, buf, fin) {
                Ok(write_size) => {
                    log::trace!(
                        "{:?} stream write succeed, stream_id={}, len={}, fin={}",
                        self,
                        id,
                        write_size,
                        fin,
                    );

                    if state.lastest_outgoing_stream_id % 2 == id % 2 {
                        // remove id from outgoing set.
                        state.register_outgoing_stream_ids.remove(&id);
                    }

                    self.handle_stream_status(&mut state)?;

                    self.notify_readable(&mut state)?;

                    return Ok(write_size);
                }
                Err(quiche::Error::Done) => {
                    self.handle_stream_status(&mut state)?;

                    self.notify_readable(&mut state)?;

                    log::trace!("{:?}, stream write Done, stream_id={}", self, id,);

                    match self.mediator.wait(event.clone(), state).await {
                        Ok(_) => {
                            log::trace!("{:?} wakeup stream to write data, stream_id={}", self, id,);

                            // try again.
                            continue;
                        }
                        Err(err) => {
                            log::error!(
                                "{:?} wakeup stream to write failed, stream_id={}, err={}",
                                self,
                                id,
                                err
                            );

                            return Err(into_io_error(err));
                        }
                    }
                }
                Err(err) => {
                    log::error!(
                        "{:?} write stream data failed, stream_id={}, err={}",
                        self,
                        id,
                        err
                    );

                    self.handle_quic_conn_status(&mut state)?;

                    return Err(into_io_error(err));
                }
            }
        }
    }

    async fn stream_recv_wait<'a, State>(
        &'a self,
        stream_id: u64,
        mut state: State,
    ) -> io::Result<()>
    where
        State: AsyncGuardMut<'a> + DerefMut<Target = RawQuicConnState> + Unpin + 'a,
    {
        self.notify_readable(&mut state)?;

        let event = QuicConnStateEvent::StreamReadable(self.scid.clone(), stream_id);

        let wait_fut = async {
            self.mediator
                .wait(event, state)
                .await
                .map_err(into_io_error)
        };

        match wait_fut.await {
            Ok(_) => {
                log::trace!(
                    "{:?} wakeup stream to read data, stream_id={}",
                    self,
                    stream_id
                );

                return Ok(());
            }
            Err(err) => {
                log::error!(
                    "{:?} wakeup stream to read failed, stream_id={}, err={}",
                    self,
                    stream_id,
                    err
                );

                return Err(into_io_error(err));
            }
        }
    }
    /// Reads data from stream, and returns tuple (read_size,fin)
    pub async fn stream_recv(&self, id: u64, buf: &mut [u8]) -> io::Result<(usize, bool)> {
        loop {
            log::trace!(
                "{:?} stream read, stream_id={}, len={}",
                self,
                id,
                buf.len(),
            );

            // Asynchronously lock the [`QuicConnState`]
            let mut state = self.state.lock().await;

            match state.quiche_conn.stream_recv(id, buf) {
                Ok((read_size, fin)) => {
                    log::trace!(
                        "{:?} stream read succeed, stream_id={}, len={}, fin={}",
                        self,
                        id,
                        read_size,
                        fin,
                    );

                    self.handle_stream_status(&mut state)?;

                    self.notify_readable(&mut state)?;

                    return Ok((read_size, fin));
                }
                Err(quiche::Error::Done) => {
                    self.handle_stream_status(&mut state)?;

                    log::trace!("{:?}, stream read Done, stream_id={}", self, id,);

                    self.stream_recv_wait(id, state).await?;
                }
                Err(quiche::Error::InvalidStreamState(_)) => {
                    // Outgoing stream is not created yet, waiting for a readable event.
                    if state.register_outgoing_stream_ids.contains(&id) {
                        log::trace!(
                            "{:?},outgoing stream read before send, stream_id={}",
                            self,
                            id,
                        );

                        self.stream_recv_wait(id, state).await?;
                    } else {
                        return Err(into_io_error(quiche::Error::InvalidStreamState(id)));
                    }
                }
                Err(err) => {
                    log::error!(
                        "{:?} write stream data failed, stream_id={}, err={}",
                        self,
                        id,
                        err
                    );

                    self.handle_stream_status(&mut state)?;

                    return Err(into_io_error(err));
                }
            }
        }
    }

    /// Accept one incoming stream.
    ///
    /// If there are no more incoming streams,the function will hang the current task,
    pub async fn accept(&self) -> Option<u64> {
        let event = QuicConnStateEvent::Accept(self.scid.clone());

        loop {
            // Asynchronously lock the [`QuicConnState`]
            let mut state = self.state.lock().await;

            if self.handle_quic_conn_status(&mut state).is_err() {
                return None;
            }

            if let Some(incoming) = state.incoming.pop_front() {
                return Some(incoming);
            }

            log::trace!("{:?} accept incoming strema pending.", self,);

            match self.mediator.wait(event.clone(), state).await {
                Ok(_) => {
                    log::trace!("{:?} wakeup accept task", self,);

                    // try again.
                    continue;
                }
                Err(err) => {
                    log::error!("{:?} wakeup accept task failed,  err={}", self, err);

                    // Safety: The event wait function returns an error message only if the event is canceled/destroyed.
                    // then it indicates that the connection is being closed or has been closed.
                    return None;
                }
            }
        }
    }

    /// Open new stream to communicate with peer.
    pub async fn open_stream(&self) -> io::Result<u64> {
        let mut state = self.state.lock().await;

        self.handle_quic_conn_status(&mut state)?;

        let stream_id = state.lastest_outgoing_stream_id;
        state.lastest_outgoing_stream_id += 4;

        state.register_outgoing_stream_ids.insert(stream_id);

        Ok(stream_id)
    }

    /// Close stream by stream `id`.
    ///
    /// This function closes stream by sending len(0) data and fin flag.
    pub async fn close_stream(&self, id: u64) -> io::Result<()> {
        let mut state = self.state.lock().await;

        self.handle_quic_conn_status(&mut state)?;

        state.dropping_stream_queue.push_back(id);

        self.notify_readable(&mut state)?;

        Ok(())
    }

    /// Shuts down reading or writing from/to the specified stream.
    ///
    /// see quiche [`doc`](https://docs.rs/quiche/latest/quiche/struct.Connection.html#method.stream_shutdown) for more information.
    pub async fn stream_shutdown(&self, stream_id: u64, err: u64) -> io::Result<()> {
        self.state
            .lock()
            .await
            .quiche_conn
            .stream_shutdown(stream_id, quiche::Shutdown::Read, err)
            .map_err(into_io_error)
    }

    /// Closes the connection with the given error and reason.
    ///
    /// see quiche [`doc`](https://docs.rs/quiche/latest/quiche/struct.Connection.html#method.close) for more information.
    pub async fn close(&self, app: bool, err: u64, reason: &[u8]) -> io::Result<()> {
        match self.state.lock().await.quiche_conn.close(app, err, reason) {
            Ok(_) => Ok(()),
            Err(quiche::Error::Done) => Ok(()),
            Err(err) => Err(into_io_error(err)),
        }
    }

    /// Returns true if all the data has been read from the specified stream.
    /// This instructs the application that all the data received from the peer on the stream has been read, and there won’t be anymore in the future.
    /// Basically this returns true when the peer either set the fin flag for the stream, or sent RESET_STREAM.
    pub async fn stream_finished(&self, stream_id: u64) -> bool {
        self.state
            .lock()
            .await
            .quiche_conn
            .stream_finished(stream_id)
    }

    /// Returns true if the connection is closed.
    pub async fn is_closed(&self) -> bool {
        self.state.lock().await.quiche_conn.is_closed()
    }

    /// Returns true if the connection is established.
    pub async fn is_established(&self) -> bool {
        self.state.lock().await.quiche_conn.is_established()
    }

    /// Returns the statistics about the connection.
    pub async fn states(&self) -> quiche::Stats {
        self.state.lock().await.quiche_conn.stats()
    }

    /// Returns the number of source Connection IDs that should be provided to the peer without exceeding the limit it advertised.
    pub async fn scids_left(&self) -> usize {
        self.state.lock().await.quiche_conn.scids_left()
    }

    /// Adjusted peer_streams_left_bidi according to register_outgoing_stream_ids.
    pub async fn peer_streams_left_bidi(&self) -> u64 {
        let state = self.state.lock().await;

        let peer_streams_left_bidi = state.quiche_conn.peer_streams_left_bidi();
        let outgoing_cached = state.register_outgoing_stream_ids.len() as u64;

        if peer_streams_left_bidi >= outgoing_cached {
            peer_streams_left_bidi - outgoing_cached
        } else {
            0
        }
    }

    /// Convert [`QuicConnState`] to [`quiche::Connection`]
    pub async fn to_quiche_conn(&self) -> impl ops::Deref<Target = quiche::Connection> + '_ {
        self.state
            .lock()
            .await
            .deref_map(|state| &state.quiche_conn)
    }
}

impl Drop for QuicConnState {
    fn drop(&mut self) {
        // The last one instance is dropping.
        if Arc::strong_count(&self.state) == 1 {
            let this = self.clone();
            future_spawn(async move {
                this.close(false, 0, b"raii drop").await.unwrap();
                // broke recursively drop
                mem::forget(this);
            });
        }
    }
}
