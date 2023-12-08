use std::{
    io,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use futures::{
    channel::mpsc::{channel, Receiver, Sender},
    SinkExt, StreamExt,
};
use quiche::ConnectionId;

use crate::errors::to_io_error;

use super::{CloseEvent, QuicConnEvent, QuicStream};

/// QuicConn represents a connected quic session.
pub struct QuicConn {
    /// The seed for stream id generator
    next_stream_id: Arc<AtomicU64>,
    /// The stream incoming data chanel buffer size.
    stream_channel_buffer: usize,
    /// This connection id
    conn_id: ConnectionId<'static>,
    /// Send create new stream event.
    conn_data_sender: Sender<QuicConnEvent>,
    /// Incoming `QuicStream` receiver.
    stream_accept_receiver: Receiver<QuicStream>,
    /// Send close event to event_loop
    close_sender: Sender<CloseEvent>,
}

impl QuicConn {
    /// Accept next quic stream.
    pub async fn accept(&mut self) -> Option<QuicStream> {
        self.stream_accept_receiver.next().await
    }

    /// Open new stream on this connection
    pub async fn open_stream(&mut self) -> io::Result<QuicStream> {
        // Open stream incoming data channel.
        let (sx, rx) = channel(self.stream_channel_buffer);

        let stream_id = self.gen_next_stream_id();

        // Send `QuicConnEvent::OpenStream` to event_loop
        self.conn_data_sender
            .send(QuicConnEvent::OpenStream {
                conn_id: self.conn_id.clone(),
                stream_id,
                sender: sx,
            })
            .await
            .map_err(to_io_error)?;

        Ok(QuicStream::new(rx, self.conn_data_sender.clone()))
    }

    /// Create new stream id
    fn gen_next_stream_id(&mut self) -> u64 {
        self.next_stream_id.fetch_add(2, Ordering::SeqCst)
    }
}

impl Drop for QuicConn {
    fn drop(&mut self) {
        self.close_sender
            .try_send(CloseEvent::Connection(self.conn_id.clone()))
            .unwrap();
    }
}
