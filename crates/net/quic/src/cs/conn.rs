use std::{fmt::Debug, io, time::Duration};

use futures::{
    channel::mpsc::{self, Receiver, Sender},
    SinkExt, StreamExt,
};

use hala_future::executor::future_spawn;
use hala_io::bytes::Bytes;
use quiche::{ConnectionId, SendInfo};

use crate::errors::into_io_error;

use super::{QuicConnCmd, QuicConnState, QuicIncomingStream, QuicStream};

/// Quic connection configuration data.
#[derive(Clone)]
struct QuicConnConfig {
    cache_len: usize,
    scid: ConnectionId<'static>,
    dcid: ConnectionId<'static>,
}

impl Debug for QuicConnConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "scid={:?}, dcid={:?}, cache_len={}",
            self.scid, self.dcid, self.cache_len
        )
    }
}

/// The incoming quic stream acceptor.
pub struct QuicStreamAcceptor {
    config: QuicConnConfig,
    incoming_stream_receiver: Receiver<QuicIncomingStream>,
    event_sender: Sender<QuicConnCmd>,
}

impl Debug for QuicStreamAcceptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "QuicConnAcceptor, {:?}", self.config,)
    }
}

impl Drop for QuicStreamAcceptor {
    fn drop(&mut self) {
        let mut event_sender = self.event_sender.clone();

        future_spawn(async move {
            _ = event_sender.send(QuicConnCmd::Close).await;
        });
    }
}

impl QuicStreamAcceptor {
    /// Accept new incoming stream.
    pub async fn accept(&mut self) -> Option<QuicStream> {
        self.incoming_stream_receiver
            .next()
            .await
            .map(|incoming_stream| {
                QuicStream::new(
                    self.config.scid.clone(),
                    self.config.dcid.clone(),
                    self.event_sender.clone(),
                    incoming_stream,
                )
            })
    }
}

/// Connector to open new outgoing bidi quic stream.
pub struct QuicStreamConnector {
    config: QuicConnConfig,
    stream_id_next: u64,
    event_sender: Sender<QuicConnCmd>,
}

impl Debug for QuicStreamConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "QuicStreamConnector, {:?}", self.config,)
    }
}

impl QuicStreamConnector {
    /// Create new bidi outgoing stream over underly connection.
    pub async fn open(&mut self) -> io::Result<QuicStream> {
        let stream_id = self.stream_id_next;

        self.stream_id_next += 2;

        let (stream_data_sender, stream_data_receiver) = mpsc::channel(self.config.cache_len);

        self.event_sender
            .send(QuicConnCmd::OpenStream {
                stream_id,
                stream_data_sender,
            })
            .await
            .map_err(into_io_error)?;

        Ok(QuicStream::new(
            self.config.scid.clone(),
            self.config.dcid.clone(),
            self.event_sender.clone(),
            QuicIncomingStream {
                stream_id,
                stream_data_receiver,
            },
        ))
    }
}

/// Quic connection type.
pub struct QuicConn {
    acceptor: QuicStreamAcceptor,
    connector: QuicStreamConnector,
}

impl Debug for QuicConn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "QuicConn, {:?}", self.acceptor.config)
    }
}

impl QuicConn {
    pub(super) fn make_client_conn(
        quiche_conn: quiche::Connection,
        send_ping_timeout: Duration,
        cache_len: usize,
    ) -> (QuicConn, QuicConnState, Receiver<(Bytes, SendInfo)>) {
        let (event_sender, event_receiver) = mpsc::channel(cache_len);

        let (conn_data_sender, conn_data_receiver) = mpsc::channel(cache_len);

        let (incoming_stream_sender, incoming_stream_receiver) = mpsc::channel(cache_len);

        let scid = quiche_conn.source_id().clone().into_owned();
        let dcid = quiche_conn.destination_id().clone().into_owned();

        let stream_id_next = 4;

        let state = QuicConnState::new(
            cache_len,
            quiche_conn,
            send_ping_timeout,
            event_receiver,
            conn_data_sender,
            incoming_stream_sender,
        );

        let config = QuicConnConfig {
            cache_len,
            scid,
            dcid,
        };

        let stream_acceptor = QuicStreamAcceptor {
            config: config.clone(),
            incoming_stream_receiver,
            event_sender: event_sender.clone(),
        };

        let stream_connector = QuicStreamConnector {
            config,
            stream_id_next,
            event_sender,
        };

        let conn = QuicConn {
            acceptor: stream_acceptor,
            connector: stream_connector,
        };

        (conn, state, conn_data_receiver)
    }

    pub(super) fn make_server_conn(
        conn_data_sender: Sender<(Bytes, SendInfo)>,
        quiche_conn: quiche::Connection,
        send_ping_timeout: Duration,
        cache_len: usize,
    ) -> (QuicConn, QuicConnState) {
        let (event_sender, event_receiver) = mpsc::channel(cache_len);

        let (incoming_stream_sender, incoming_stream_receiver) = mpsc::channel(cache_len);

        let scid = quiche_conn.source_id().clone().into_owned();
        let dcid = quiche_conn.destination_id().clone().into_owned();

        let stream_id_next = 5;

        let state = QuicConnState::new(
            cache_len,
            quiche_conn,
            send_ping_timeout,
            event_receiver,
            conn_data_sender,
            incoming_stream_sender,
        );

        let config = QuicConnConfig {
            cache_len,
            scid,
            dcid,
        };

        let stream_acceptor = QuicStreamAcceptor {
            config: config.clone(),
            incoming_stream_receiver,
            event_sender: event_sender.clone(),
        };

        let stream_connector = QuicStreamConnector {
            config,
            stream_id_next,
            event_sender,
        };

        let conn = QuicConn {
            acceptor: stream_acceptor,
            connector: stream_connector,
        };

        (conn, state)
    }
}

impl QuicConn {
    /// Accept new incoming stream.
    pub async fn accept(&mut self) -> Option<QuicStream> {
        self.acceptor.accept().await
    }

    /// Open a new bidi stream.
    pub async fn open(&mut self) -> io::Result<QuicStream> {
        self.connector.open().await
    }

    /// Split connection into two half part: [`acceptor`](QuicStreamAcceptor) and [`connector`](QuicStreamConnector)
    pub fn split(self) -> (QuicStreamAcceptor, QuicStreamConnector) {
        (self.acceptor, self.connector)
    }

    /// Reunite quic connection from two half part: [`acceptor`](QuicStreamAcceptor) and [`connector`](QuicStreamConnector)
    pub fn reunite(acceptor: QuicStreamAcceptor, connector: QuicStreamConnector) -> Self {
        Self {
            acceptor,
            connector,
        }
    }

    /// Get a clone of [`QuicConnCmd`] sender.
    pub(super) fn event_sender(&self) -> Sender<QuicConnCmd> {
        self.acceptor.event_sender.clone()
    }

    /// Get quic connection source id
    pub fn source_id(&self) -> &ConnectionId<'static> {
        &self.acceptor.config.scid
    }

    /// Get quic connection destination id
    pub fn destination_id(&self) -> &ConnectionId<'static> {
        &self.acceptor.config.dcid
    }
}
