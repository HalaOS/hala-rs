use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
    io,
    time::{Duration, Instant},
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
use quiche::{RecvInfo, SendInfo};

use crate::errors::into_io_error;

use super::{QuicIncomingStream, QuicStreamBuf};

/// Event variant for quic connection state machine.
pub(super) enum QuicConnCmd {
    OpenStream {
        /// Unique stream ID
        stream_id: u64,
        /// Stream received data sender.
        stream_data_sender: Sender<QuicStreamBuf>,
    },

    StreamSend {
        /// Unique stream ID
        stream_id: u64,
        /// Stream data buf to send.
        buf: BytesMut,
        /// Stream send fin flag.
        fin: bool,
    },

    Recv {
        /// The received data from peer for this connection.
        buf: BytesMut,
        /// Recv information.
        recv_info: RecvInfo,
    },

    Close,
}

/// Quic conn state machine server side.
pub struct QuicConnState {
    /// The max length of stream received data cache queue.
    max_stream_recv_packet_cache_len: usize,
    /// quiche connection instance.
    quiche_conn: quiche::Connection,
    /// Event receiver of state machine
    event_receiver: Receiver<QuicConnCmd>,
    /// Quic connection data sender.
    conn_data_sender: Sender<(Bytes, SendInfo)>,
    /// Incoming stream sender.
    incoming_stream_sender: Sender<QuicIncomingStream>,
    /// Incoming stream register to registers all seen stream IDs.
    incoming_stream_ids: HashSet<u64>,
    /// Timestamp for lastest sent ack eliciting.
    send_ack_eliciting_instant: Instant,
    /// Interval for send ping packet.
    send_ping_timeout: Duration,
    /// Quic stream data senders.
    stream_data_senders: HashMap<u64, Sender<QuicStreamBuf>>,
    /// The cached stream send bufs.
    stream_sending_bufs: HashMap<u64, QuicStreamBuf>,
}

impl Debug for QuicConnState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "QuicConnState, scid={:?}, dcid={:?}, is_server={}",
            self.quiche_conn.source_id(),
            self.quiche_conn.destination_id(),
            self.quiche_conn.is_server()
        )
    }
}

impl QuicConnState {
    /// Create new quic connection state machine. and ready to run.
    pub(super) fn new(
        max_stream_recv_packet_cache_len: usize,
        quiche_conn: quiche::Connection,
        send_ping_timeout: Duration,
        event_receiver: Receiver<QuicConnCmd>,
        conn_data_sender: Sender<(Bytes, SendInfo)>,
        incoming_stream_sender: Sender<QuicIncomingStream>,
    ) -> Self {
        Self {
            max_stream_recv_packet_cache_len,
            quiche_conn,
            event_receiver,
            conn_data_sender,
            incoming_stream_sender,
            incoming_stream_ids: Default::default(),
            stream_data_senders: Default::default(),
            send_ping_timeout,
            send_ack_eliciting_instant: Instant::now(),
            stream_sending_bufs: Default::default(),
        }
    }

    /// Consumer and run quic connection state machine.
    ///
    /// This function will not block current task process
    pub fn start(mut self) {
        future_spawn(async move {
            log::trace!("{:?}, started", self);

            match self.event_loop().await {
                Ok(_) => {
                    log::info!("{:?}, stopped", self);
                }
                Err(err) => {
                    log::info!("{:?}, stopped due to error, {}", self, err);
                }
            }
        });
    }

    async fn send(&mut self) -> io::Result<()> {
        let mut buf = ReadBuf::with_capacity(self.quiche_conn.send_quantum());

        match self.quiche_conn.send(buf.as_mut()) {
            Ok((send_size, send_info)) => {
                let buf = buf.into_bytes(Some(send_size));

                log::trace!(
                    "{:?}, send data, send_size={}, elapsed={:?}",
                    self,
                    send_size,
                    send_info.at.elapsed()
                );

                self.conn_data_sender
                    .send((buf, send_info))
                    .await
                    .map_err(into_io_error)?;
            }
            Err(quiche::Error::Done) => {}
            Err(err) => {
                return Err(into_io_error(err));
            }
        }

        Ok(())
    }

    /// Calculate send ping/timeout data duration.
    async fn handle_send_timout(&mut self) -> io::Result<Duration> {
        loop {
            let mut elapsed = self.send_ack_eliciting_instant.elapsed();

            while elapsed >= self.send_ping_timeout {
                self.quiche_conn
                    .send_ack_eliciting()
                    .map_err(into_io_error)?;

                // reset ping timeout
                self.send_ack_eliciting_instant = Instant::now();

                self.send().await?;

                elapsed = self.send_ack_eliciting_instant.elapsed();
            }

            let send_ping_timeout = self.send_ping_timeout - elapsed;

            if let Some(timeout) = self.quiche_conn.timeout() {
                if timeout < send_ping_timeout {
                    if self.quiche_conn.timeout_instant().unwrap() < Instant::now() {
                        self.send().await?;

                        // The above code may be taking too long to execute, so it needs to be re-run to check.
                        continue;
                    }

                    return Ok(timeout);
                }
            }

            return Ok(send_ping_timeout);
        }
    }

    /// Start main event loop
    async fn event_loop(&mut self) -> io::Result<()> {
        loop {
            let block_timeout = self.handle_send_timout().await?;

            let next = self.event_receiver.next();

            match timeout(async { Ok(next.await) }, Some(block_timeout)).await {
                Ok(Some(event)) => {
                    self.dispatch_event(event).await?;
                }
                // The connection state machine stops normally.
                Ok(None) => {
                    log::trace!("{:?}, connection dropped", self);
                    break;
                }
                Err(err) if err.kind() == io::ErrorKind::TimedOut => {
                    if let Some(timeout_instant) = self.quiche_conn.timeout_instant() {
                        if timeout_instant < Instant::now() {
                            log::trace!("{:?}, timeout", self);

                            self.quiche_conn.timeout();
                        }
                    }

                    if self.send_ack_eliciting_instant.elapsed() >= self.send_ping_timeout {
                        log::trace!("{:?}, send ping packet", self);

                        self.quiche_conn
                            .send_ack_eliciting()
                            .map_err(into_io_error)?;

                        self.send_ack_eliciting_instant = Instant::now();
                    }

                    self.send().await?;
                }
                // break conn state machine.
                Err(err) => return Err(err),
            }
        }

        Ok(())
    }

    /// dispach received state machine event.
    async fn dispatch_event(&mut self, event: QuicConnCmd) -> io::Result<()> {
        match event {
            QuicConnCmd::OpenStream {
                stream_id,
                stream_data_sender,
            } => self.on_open_stream(stream_id, stream_data_sender).await,
            QuicConnCmd::StreamSend {
                stream_id,
                buf,
                fin,
            } => {
                if let Some(send) = self.stream_sending_bufs.get_mut(&stream_id) {
                    send.append(buf, fin);

                    Ok(())
                } else {
                    self.on_stream_send(stream_id, buf, fin).await
                }
            }
            QuicConnCmd::Recv { buf, recv_info } => self.on_recv(buf, recv_info).await,
            QuicConnCmd::Close => {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Quic conn state machine canceld by user.",
                ));
            }
        }
    }

    async fn on_incoming_stream(&mut self, stream_id: u64) -> io::Result<()> {
        // check if incoming stream id.
        if (self.quiche_conn.is_server() && stream_id % 2 == 0)
            || (!self.quiche_conn.is_server() && stream_id % 2 == 1)
        {
            // new incoming stream.
            if !self.incoming_stream_ids.insert(stream_id) {
                let (stream_data_sender, stream_data_receiver) =
                    mpsc::channel(self.max_stream_recv_packet_cache_len);

                assert!(
                    self.stream_data_senders
                        .insert(stream_id, stream_data_sender)
                        .is_none(),
                    "register incoming stream({stream_id}) twice"
                );

                let incoming_stream = QuicIncomingStream {
                    stream_id,
                    stream_data_receiver,
                };

                self.incoming_stream_sender
                    .send(incoming_stream)
                    .await
                    .map_err(into_io_error)?;
            }
        }

        Ok(())
    }

    /// process stream recv/send ops.
    async fn dispatch_stream_datas(&mut self) -> io::Result<()> {
        for stream_id in self.quiche_conn.writable() {
            if let Some(send) = self.stream_sending_bufs.remove(&stream_id) {
                self.on_stream_send(stream_id, send.buf, send.fin).await?;
            }
        }

        for stream_id in self.quiche_conn.readable() {
            self.on_incoming_stream(stream_id).await?;
            self.on_stream_recv(stream_id).await?;
        }

        todo!()
    }

    /// Handle connection recevied data.
    async fn on_recv(&mut self, mut buf: BytesMut, recv_info: RecvInfo) -> io::Result<()> {
        self.quiche_conn
            .recv(&mut buf, recv_info)
            .map_err(into_io_error)?;

        self.dispatch_stream_datas().await?;

        Ok(())
    }

    /// Handle open stream event.
    async fn on_open_stream(
        &mut self,
        stream_id: u64,
        stream_data_sender: Sender<QuicStreamBuf>,
    ) -> io::Result<()> {
        assert!(
            self.stream_data_senders
                .insert(stream_id, stream_data_sender)
                .is_none(),
            "Call open stream({stream_id}) twice"
        );

        Ok(())
    }

    /// Check stream status and remove stream resources.
    fn try_close_stream(&mut self, stream_id: u64) {
        if self.quiche_conn.stream_finished(stream_id) {
            self.stream_data_senders.remove(&stream_id);
            self.stream_sending_bufs.remove(&stream_id);
        }
    }

    /// cache stream send buf for next try.
    fn cache_stream_send_remaining(&mut self, stream_id: u64, buf: BytesMut, fin: bool) {
        if let Some(send) = self.stream_sending_bufs.get_mut(&stream_id) {
            send.append(buf, fin);
        } else {
            self.stream_sending_bufs
                .insert(stream_id, QuicStreamBuf { buf, fin });
        }
    }

    async fn on_stream_recv(&mut self, stream_id: u64) -> io::Result<()> {
        let mut buf = ReadBuf::with_capacity(self.quiche_conn.send_quantum());

        match self.quiche_conn.stream_recv(stream_id, buf.as_mut()) {
            Ok((recv_size, fin)) => {
                log::trace!(
                    "{:?}, stream_id={}, recv_size={}, fin={}, recv data successfully",
                    self,
                    stream_id,
                    recv_size,
                    fin,
                );

                let buf = buf.into_bytes_mut(Some(recv_size));

                // Reading data from a stream may trigger queueing of control messages (e.g. MAX_STREAM_DATA).
                // send() should be called after reading.
                self.send().await?;

                if let Some(sender) = self.stream_data_senders.get_mut(&stream_id) {
                    sender
                        .send(QuicStreamBuf { buf, fin })
                        .await
                        .map_err(into_io_error)?;
                } else {
                    assert!(false, "stream({stream_id}) has been removed prematurely");
                }
            }
            Err(quiche::Error::Done) => {
                assert!(false, "not here");
            }
            Err(err) => {
                log::error!(
                    "{:?}, stream_id={}, recv data error: {}",
                    self,
                    stream_id,
                    err
                );
                return Err(into_io_error(err));
            }
        }

        Ok(())
    }

    async fn on_stream_send(
        &mut self,
        stream_id: u64,
        mut buf: BytesMut,
        fin: bool,
    ) -> io::Result<()> {
        assert!(
            self.stream_data_senders.contains_key(&stream_id),
            "Call stream({stream_id}) send before open it."
        );

        let stream_capacity = match self.quiche_conn.stream_capacity(stream_id) {
            Ok(capacity) => capacity,
            Err(quiche::Error::StreamStopped(_)) => {
                log::error!(
                    "{:?}, stream_id={}, stream stopped by STOP_SENDING",
                    self,
                    stream_id,
                );

                self.try_close_stream(stream_id);

                return Ok(());
            }
            Err(err) => {
                log::error!(
                    "{:?}, stream_id={}, send data error: {}",
                    self,
                    stream_id,
                    err
                );
                return Err(into_io_error(err));
            }
        };

        if stream_capacity == 0 {
            self.cache_stream_send_remaining(stream_id, buf, fin);

            return Ok(());
        }

        let remaining = buf.split_off(stream_capacity);

        // The fin flag is set only when the entire buf has been transferred.
        let send_fin = if remaining.len() == 0 { fin } else { false };

        match self.quiche_conn.stream_send(stream_id, &buf, send_fin) {
            Ok(send_size) => {
                assert_eq!(send_size, buf.len());

                log::trace!(
                    "{:?}, stream_id={}, send data, len={}",
                    self,
                    stream_id,
                    send_size,
                );

                // send connection data immediately
                self.send().await?;

                self.cache_stream_send_remaining(stream_id, buf, fin);
            }
            Err(quiche::Error::Done) => {
                assert!(false, "not here");
            }
            Err(quiche::Error::StreamStopped(_)) => {
                log::error!(
                    "{:?}, stream_id={}, stream stopped by STOP_SENDING",
                    self,
                    stream_id,
                );

                self.try_close_stream(stream_id);

                return Ok(());
            }
            Err(err) => {
                log::error!(
                    "{:?}, stream_id={}, send data error: {}",
                    self,
                    stream_id,
                    err
                );
                return Err(into_io_error(err));
            }
        }

        Ok(())
    }
}
